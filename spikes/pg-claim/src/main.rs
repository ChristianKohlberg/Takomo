//! Does `SELECT ... FOR UPDATE SKIP LOCKED` actually give Takomo's
//! exactly-one-claimant guarantee on Postgres, with the writer mutex gone?
//!
//! Today that guarantee is inherited, not proven: `Store::with_tx` serializes
//! every mutation behind a process-wide `Mutex<Connection>` and one SQLite
//! `IMMEDIATE` transaction, so two workers cannot both win a ticket because two
//! writers cannot exist. Dropping the mutex is the entire point of moving to
//! Postgres — it is what allows two app instances against one database — and it
//! means the guarantee has to be re-established rather than assumed.
//!
//! So this runs both arms against a real Postgres:
//!
//!   NAIVE   SELECT the head of the ready queue, then UPDATE it. What you get if
//!           you port the SQL literally and delete the mutex. Expected to
//!           double-claim.
//!   LOCKED  the same SELECT with `FOR UPDATE OF t SKIP LOCKED`. Expected to
//!           hand each ticket to exactly one worker.
//!
//! The negative control is the point. An assertion that only ever passes proves
//! nothing about whether the mechanism is load-bearing; showing the race is real
//! and then showing it gone is what makes the green arm mean something.
//!
//! Run:  docker run -d --name takomo-pg-spike -e POSTGRES_PASSWORD=takomo \
//!         -e POSTGRES_USER=takomo -e POSTGRES_DB=takomo -p 55432:5432 postgres:16-alpine
//!       cargo run --release

use sqlx::postgres::{PgPoolOptions, PgRow};
use sqlx::{Executor, PgPool, Row};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

const TICKETS: i32 = 60;
const WORKERS: usize = 16;

/// The ready-queue predicate from `ready_scope` in `src/store/claims.rs`,
/// translated. Two changes a reader should not have to hunt for:
///   * `bs.terminal = 0` / `ws.claimable = 1` become plain booleans (schema rule 2)
///   * `t.rowid` becomes `t.seq` (schema rule 1)
/// Everything else — the recursive `blocked` CTE, the claim-expiry check, the
/// occurrence-expiry check, the priority ordering — is the live predicate.
const READY: &str = r#"
WITH RECURSIVE blocked(id) AS (
    SELECT DISTINCT d.ticket
    FROM deps d
    JOIN tickets b ON b.id = d.blocked_by
    JOIN workflow_states bs ON bs.project = b.project AND bs.state = b.state
    WHERE NOT bs.terminal
    UNION
    SELECT c.id FROM tickets c JOIN blocked ON c.parent = blocked.id
)
SELECT t.id FROM tickets t
JOIN workflow_states ws ON ws.project = t.project AND ws.state = t.state
WHERE ws.claimable
  AND t.archived_at IS NULL
  AND (t.claim_holder IS NULL OR t.claim_expires_at <= $1)
  AND (t.expires_at IS NULL OR t.expires_at > $1)
  AND t.id NOT IN (SELECT id FROM blocked)
ORDER BY CASE t.priority
           WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'normal' THEN 2 ELSE 3
         END,
         t.created_at ASC,
         t.seq ASC
"#;

#[derive(Clone, Copy, PartialEq)]
enum Arm {
    Naive,
    Locked,
}

impl Arm {
    fn sql(self) -> String {
        match self {
            // A literal port of today's SQL with the mutex removed.
            Arm::Naive => format!("{READY} LIMIT 1"),
            // `OF t` matters: the query joins workflow_states, and without it
            // Postgres would try to lock the workflow_states row too — which is
            // shared by every ticket in the project and would serialize the
            // whole queue back down to one worker.
            Arm::Locked => format!("{READY} FOR UPDATE OF t SKIP LOCKED LIMIT 1"),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Arm::Naive => "NAIVE  (SELECT then UPDATE, no row lock)",
            Arm::Locked => "LOCKED (FOR UPDATE OF t SKIP LOCKED)",
        }
    }
}

async fn reset(pool: &PgPool) -> sqlx::Result<()> {
    pool.execute(
        r#"
        DROP TABLE IF EXISTS events, deps, tickets, workflow_states, projects CASCADE;
        "#,
    )
    .await?;
    let schema = include_str!("../../../migrations/postgres/0001_core.sql");
    pool.execute(schema).await?;

    pool.execute(
        r#"
        INSERT INTO projects (id, name, workflow_json, created_at)
        VALUES ('demo', 'Demo', '{}', 0);
        INSERT INTO workflow_states (project, state, category, claimable, terminal) VALUES
          ('demo', 'ready', 'open',   TRUE,  FALSE),
          ('demo', 'done',  'closed', FALSE, TRUE);
        "#,
    )
    .await?;

    // Every ticket claimable, none blocked: maximum contention, which is the
    // condition the guarantee has to hold under.
    let mut q = String::from(
        "INSERT INTO tickets (id, project, state, title, created_by, created_at, updated_at) VALUES ",
    );
    for i in 0..TICKETS {
        if i > 0 {
            q.push(',');
        }
        q.push_str(&format!(
            "('t{i}', 'demo', 'ready', 'ticket {i}', 'seed', 0, 0)"
        ));
    }
    pool.execute(q.as_str()).await?;
    Ok(())
}

/// One worker: claim until the queue is empty, recording what it won.
async fn worker(pool: PgPool, arm: Arm, won: Arc<Mutex<Vec<String>>>, actor: String) {
    let sql = arm.sql();
    loop {
        let mut tx = match pool.begin().await {
            Ok(t) => t,
            Err(_) => return,
        };

        let row: Option<PgRow> = sqlx::query(&sql)
            .bind(0_i64) // `now`; every ticket's expiries are NULL in this fixture
            .fetch_optional(&mut *tx)
            .await
            .unwrap_or(None);

        let Some(row) = row else {
            // Nothing claimable. With SKIP LOCKED this also means "nothing that
            // isn't already locked by a peer", so a worker can retire early
            // while others still hold rows — that is correct, not a miss.
            let _ = tx.rollback().await;
            return;
        };
        let id: String = row.get("id");

        // Widen the window between decide and write. Under the mutex this window
        // does not exist at all; on Postgres it is exactly where a second
        // claimant gets in, so the experiment has to make it observable rather
        // than hope the scheduler produces it.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;

        let updated = sqlx::query(
            "UPDATE tickets SET fence_seq = fence_seq + 1, claim_holder = $2, \
             claim_expires_at = 9999999999, lapsed_claim_holder = NULL, \
             version = version + 1, updated_at = 1 WHERE id = $1",
        )
        .bind(&id)
        .bind(&actor)
        .execute(&mut *tx)
        .await;

        if updated.is_ok() && tx.commit().await.is_ok() {
            won.lock().await.push(id);
        }
    }
}

async fn run(pool: &PgPool, arm: Arm) -> sqlx::Result<()> {
    reset(pool).await?;

    let won = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for w in 0..WORKERS {
        handles.push(tokio::spawn(worker(
            pool.clone(),
            arm,
            won.clone(),
            format!("worker-{w}"),
        )));
    }
    for h in handles {
        let _ = h.await;
    }

    let claims = won.lock().await.clone();

    // Invariant 1: no ticket handed to two workers.
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for id in &claims {
        *counts.entry(id.as_str()).or_default() += 1;
    }
    let doubled: Vec<_> = counts.iter().filter(|(_, &n)| n > 1).collect();

    // Invariant 2: fence_seq is the store's own record of how many times a
    // ticket was claimed. Anything but 1 is a double-claim the bookkeeping in
    // invariant 1 could in principle have missed.
    let bad_fence: i64 = sqlx::query("SELECT COUNT(*) AS n FROM tickets WHERE fence_seq <> 1")
        .fetch_one(pool)
        .await?
        .get("n");

    // Invariant 3: every ticket got claimed by someone. SKIP LOCKED must not
    // cause a ticket to be permanently passed over.
    let unclaimed: i64 = sqlx::query("SELECT COUNT(*) AS n FROM tickets WHERE claim_holder IS NULL")
        .fetch_one(pool)
        .await?
        .get("n");

    println!("\n=== {} ===", arm.name());
    println!("  tickets                 {TICKETS}");
    println!("  workers                 {WORKERS}");
    println!("  successful claims       {}", claims.len());
    println!("  tickets claimed twice+  {}", doubled.len());
    println!("  fence_seq <> 1          {bad_fence}");
    println!("  never claimed           {unclaimed}");

    let ok = doubled.is_empty() && bad_fence == 0 && unclaimed == 0 && claims.len() == TICKETS as usize;
    println!(
        "  VERDICT                 {}",
        if ok {
            "exactly-one-claimant HOLDS"
        } else {
            "exactly-one-claimant VIOLATED"
        }
    );
    if !doubled.is_empty() {
        let mut sample: Vec<_> = doubled.iter().map(|(id, n)| format!("{id}x{n}")).collect();
        sample.sort();
        sample.truncate(8);
        println!("  sample of doubled       {}", sample.join(" "));
    }
    Ok(())
}

/// What does keeping the process-wide writer mutex COST once the database is on
/// the other end of a socket?
///
/// Under SQLite the mutex is nearly free: a write is a local page-cache write,
/// tens of microseconds, so serializing every mutation behind one lock costs
/// almost nothing. Over Postgres the same lock is held across a full
/// BEGIN/UPDATE/COMMIT round trip. That is the number that decides whether
/// "keep the mutex" is a real option or a throughput ceiling in disguise.
///
/// Serial = what a retained mutex gives you (one write at a time, end to end).
/// Pooled = what dropping it gives you (16 concurrent writers).
async fn bench(pool: &PgPool) -> sqlx::Result<()> {
    reset(pool).await?;
    const N: usize = 300;

    let t0 = std::time::Instant::now();
    for i in 0..N {
        let mut tx = pool.begin().await?;
        sqlx::query("UPDATE tickets SET version = version + 1, updated_at = $1 WHERE id = 't0'")
            .bind(i as i64)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }
    let serial = t0.elapsed();

    let t1 = std::time::Instant::now();
    let mut hs = Vec::new();
    for w in 0..WORKERS {
        let p = pool.clone();
        hs.push(tokio::spawn(async move {
            for i in 0..(N / WORKERS) {
                let Ok(mut tx) = p.begin().await else { return };
                let _ = sqlx::query(
                    "UPDATE tickets SET version = version + 1, updated_at = $1 WHERE id = $2",
                )
                .bind(i as i64)
                .bind(format!("t{w}"))
                .execute(&mut *tx)
                .await;
                let _ = tx.commit().await;
            }
        }));
    }
    for h in hs {
        let _ = h.await;
    }
    let pooled = t1.elapsed();

    println!("\n=== cost of keeping the writer mutex, over a socket ===");
    println!(
        "  serial  (mutex retained)  {N} txns in {:>7.0?}  = {:>6.2} ms/txn  ~{:>5.0} writes/s",
        serial,
        serial.as_secs_f64() * 1000.0 / N as f64,
        N as f64 / serial.as_secs_f64()
    );
    println!(
        "  pooled  (mutex dropped)   {N} txns in {:>7.0?}  = {:>6.2} ms/txn  ~{:>5.0} writes/s",
        pooled,
        pooled.as_secs_f64() * 1000.0 / N as f64,
        N as f64 / pooled.as_secs_f64()
    );
    Ok(())
}

#[tokio::main]
async fn main() -> sqlx::Result<()> {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://takomo:takomo@localhost:55432/takomo".into());
    let pool = PgPoolOptions::new()
        .max_connections(WORKERS as u32 + 2)
        .connect(&url)
        .await?;

    run(&pool, Arm::Naive).await?;
    run(&pool, Arm::Locked).await?;
    bench(&pool).await?;
    println!();
    Ok(())
}
