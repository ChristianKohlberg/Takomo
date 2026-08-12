//! The shared integration-test harness.
//!
//! `tests/api.rs` and `tests/mcp.rs` are separate test binaries, so each one
//! `mod common;`s this file and gets its own copy. Everything here is about
//! driving the *real* server: `TestApp::spawn()` opens a temp SQLite DB, mints
//! the four standard tokens, and serves on an ephemeral port, so the tests talk
//! HTTP over reqwest rather than poking `Store` directly.
//!
//! Not every helper is used by both binaries, hence the blanket `dead_code`
//! allow — an unused helper in one binary is not a defect.
#![allow(dead_code)]

use reqwest::{Method, RequestBuilder, StatusCode};
use serde_json::{json, Value};
use std::time::Duration;
use takomo::api::oauth::OauthConfig;
use takomo::server::{build_router, spawn_sweeper, AppState};
use takomo::store::Store;

pub struct TestApp {
    pub base: String,
    /// read,write,human,admin on all projects.
    pub admin: String,
    /// read,write,human.
    pub human: String,
    /// read,write (agent:w1).
    pub worker: String,
    /// read,write (agent:w2).
    pub worker2: String,
    pub client: reqwest::Client,
    /// Holds the temp dir open for the life of the app; also where `db_path`
    /// and `open_store` find the live SQLite file.
    pub tmp: tempfile::TempDir,
    /// Set when running against Postgres: the schema this app's tables live in,
    /// so `open_store` reconnects to the SAME database rather than opening a
    /// fresh empty one.
    pub pg_schema: Option<String>,
}

use takomo::store::sql::Value as SqlValue;

fn v_text(s: impl Into<String>) -> SqlValue {
    SqlValue::Text(s.into())
}
fn v_int(i: i64) -> SqlValue {
    SqlValue::Integer(i)
}

fn scope_vec(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

impl TestApp {
    pub async fn spawn() -> TestApp {
        TestApp::spawn_with(Some(Duration::from_millis(250)), false).await
    }

    /// An app with **no** lease sweeper, so a lapsed lease stays recorded on the
    /// ticket instead of being cleared within a tick. The only way to observe
    /// the expired-but-still-recorded claim deterministically — with the sweeper
    /// running, whether a call sees that row is a race against a 250ms timer.
    pub async fn spawn_without_sweeper() -> TestApp {
        TestApp::spawn_with(None, false).await
    }

    /// An app whose OAuth authorization server is configured, its issuer set to
    /// the loopback origin it actually ends up listening on.
    ///
    /// Which is why this variant binds the listener *before* building the state: a
    /// real client compares the issuer and the `resource` identifier byte for byte
    /// against what it fetched and what the user typed, so a test that invented
    /// them would pass while the deployment failed. The ephemeral port is only
    /// knowable after the bind.
    pub async fn spawn_with_oauth() -> TestApp {
        TestApp::spawn_with(Some(Duration::from_millis(250)), true).await
    }

    async fn spawn_with(sweep: Option<Duration>, oauth: bool) -> TestApp {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Differential testing: with TAKOMO_TEST_PG set, this same suite runs
        // against Postgres instead of SQLite. Same tests, same assertions — the
        // only honest way to claim the backends behave alike. Each TestApp gets
        // its own schema because the suite runs in parallel and every case
        // assumes a private database.
        let mut pg_schema = None;
        let store = match std::env::var("TAKOMO_TEST_PG") {
            Ok(url) if !url.is_empty() => {
                use std::sync::atomic::{AtomicU64, Ordering};
                static N: AtomicU64 = AtomicU64::new(0);
                let schema = format!(
                    "t{}_{}",
                    std::process::id(),
                    N.fetch_add(1, Ordering::Relaxed)
                );
                let s = Store::connect_pg_in(&url, &schema).expect("connect postgres");
                pg_schema = Some(schema);
                s
            }
            _ => Store::open(tmp.path().join("test.db")).expect("open store"),
        };
        store
            .create_project("tp", "Test Project", None, "test:setup")
            .expect("create project");
        let (_, admin) = store
            .create_token(
                "human:admin",
                &scope_vec(&["read", "write", "human", "admin"]),
                None,
                10_000,
                None,
            )
            .unwrap();
        let (_, human) = store
            .create_token(
                "human:reviewer",
                &scope_vec(&["read", "write", "human"]),
                None,
                10_000,
                None,
            )
            .unwrap();
        let (_, worker) = store
            .create_token(
                "agent:w1",
                &scope_vec(&["read", "write"]),
                None,
                10_000,
                None,
            )
            .unwrap();
        let (_, worker2) = store
            .create_token(
                "agent:w2",
                &scope_vec(&["read", "write"]),
                None,
                10_000,
                None,
            )
            .unwrap();

        // Bound first so the OAuth issuer can be the real origin; see
        // `spawn_with_oauth`.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        let oauth = oauth.then(|| {
            OauthConfig::from_public_url(&base).expect("a loopback origin is a valid public URL")
        });

        let state = AppState::new_with_oauth(store, oauth);
        if let Some(interval) = sweep {
            spawn_sweeper(state.clone(), interval);
        }
        let router = build_router(state);
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        TestApp {
            base,
            admin,
            human,
            worker,
            worker2,
            client: reqwest::Client::new(),
            tmp,
            pg_schema,
        }
    }

    // --- raw request plumbing ------------------------------------------------

    /// Absolute URL for `path` on the running server.
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// A request builder with **no** Authorization header — for the
    /// unauthenticated pages and for tests that need the raw response
    /// (status line, headers, non-JSON body).
    pub fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.client.request(method, self.url(path))
    }

    /// The application's JavaScript, as a browser receives it.
    ///
    /// The four surfaces used to be four self-contained documents, so a test
    /// about what a page ships could read `GET /board`. They are ONE
    /// client-side-routed app now: every page route returns the same small
    /// shell, and everything a page-content assertion cares about — the string
    /// tables, the typeahead mounts, the API paths the client calls — lives in
    /// the bundle that shell references.
    ///
    /// So assertions of the form "the application ships X" are unchanged in
    /// substance; only the place to look moved. Assertions about which SURFACE
    /// ships X are gone, and correctly so: there is one document now, and a
    /// test claiming `/inbox` carries something `/board` does not would be
    /// asserting a distinction that no longer exists.
    pub async fn app_bundle(&self) -> String {
        let resp = self
            .request(Method::GET, "/assets/app.js")
            .send()
            .await
            .expect("the app bundle should be served");
        assert!(
            resp.status().is_success(),
            "GET /assets/app.js returned {} — the page shell references it, so a \
             non-200 here means the app does not load at all",
            resp.status()
        );
        resp.text().await.unwrap()
    }

    /// Same as [`TestApp::request`], carrying a bearer token.
    pub fn authed(&self, method: Method, token: &str, path: &str) -> RequestBuilder {
        self.request(method, path).bearer_auth(token)
    }

    /// Send a built request and decode the JSON body (`Null` when there is none,
    /// e.g. a 204).
    pub async fn json(&self, req: RequestBuilder) -> (StatusCode, Value) {
        let resp = req.send().await.expect("request");
        let status = resp.status();
        let value = resp.json::<Value>().await.unwrap_or(Value::Null);
        (status, value)
    }

    // --- JSON verbs ----------------------------------------------------------

    pub async fn post(&self, token: &str, path: &str, body: Value) -> (StatusCode, Value) {
        self.post_with(token, path, &[], body).await
    }

    /// POST with extra request headers (`Idempotency-Key`, …).
    pub async fn post_with(
        &self,
        token: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Value,
    ) -> (StatusCode, Value) {
        let mut req = self.authed(Method::POST, token, path).json(&body);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        self.json(req).await
    }

    pub async fn get(&self, token: &str, path: &str) -> (StatusCode, Value) {
        self.json(self.authed(Method::GET, token, path)).await
    }

    /// GET returning the raw response (status, content-type, body text) — used
    /// for the JSONL export endpoint, which is not a single JSON document.
    pub async fn get_raw(&self, token: &str, path: &str) -> (StatusCode, String, String) {
        let resp = self
            .authed(Method::GET, token, path)
            .send()
            .await
            .expect("request");
        let status = resp.status();
        let ctype = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let text = resp.text().await.unwrap_or_default();
        (status, ctype, text)
    }

    /// GET returning the response bytes and one named header.
    ///
    /// [`TestApp::get_raw`] cannot serve here: the SQLite export is a binary
    /// file, and `resp.text()` would lossily replace every byte that is not
    /// valid UTF-8 — including, on a database of any size, the page bytes an
    /// assertion about validity depends on.
    pub async fn get_bytes(
        &self,
        token: &str,
        path: &str,
        header: &str,
    ) -> (StatusCode, String, Vec<u8>) {
        let resp = self
            .authed(Method::GET, token, path)
            .send()
            .await
            .expect("request");
        let status = resp.status();
        let value = resp
            .headers()
            .get(header)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let bytes = resp.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
        (status, value, bytes)
    }

    pub async fn patch(&self, token: &str, path: &str, body: Value) -> (StatusCode, Value) {
        self.patch_with(token, path, &[], body).await
    }

    /// PATCH with extra request headers — `If-Match` above all, which plain
    /// [`TestApp::patch`] cannot express.
    pub async fn patch_with(
        &self,
        token: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Value,
    ) -> (StatusCode, Value) {
        let mut req = self.authed(Method::PATCH, token, path).json(&body);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        self.json(req).await
    }

    pub async fn put(&self, token: &str, path: &str, body: Value) -> (StatusCode, Value) {
        self.json(self.authed(Method::PUT, token, path).json(&body))
            .await
    }

    pub async fn delete(&self, token: &str, path: &str) -> (StatusCode, Value) {
        self.json(self.authed(Method::DELETE, token, path)).await
    }

    // --- the live database ---------------------------------------------------

    /// The live server's SQLite file. Tests that must seed states the API
    /// deliberately refuses to create — a dangling `parent`, a `parent` cycle —
    /// write them here directly with foreign keys off, so the row lands exactly
    /// as a corrupted or hand-edited database would have it.
    pub fn db_path(&self) -> std::path::PathBuf {
        self.tmp.path().join("test.db")
    }

    /// Open a second connection to the running server's DB (WAL allows it) — used
    /// to mint a backdated/expired share without waiting on wall-clock time.

    /// True when this app is backed by Postgres rather than SQLite.
    ///
    /// A handful of tests assert SQLite mechanics specifically — `PRAGMA
    /// table_info`, opening the `.db` file as a snapshot. Those are not
    /// portability gaps to be fixed but statements about the SQLite backend, so
    /// they skip rather than fail when the suite runs against Postgres. Every
    /// such skip is spelled out at the call site.
    pub fn is_pg(&self) -> bool {
        self.pg_schema.is_some()
    }

    /// The columns `table` actually has, on whichever backend is live.
    ///
    /// `seq` is filtered out on Postgres deliberately. It is the explicit
    /// replacement for SQLite's implicit `rowid`, and `rowid` never appears in
    /// `PRAGMA table_info` either — so including it would make the two backends
    /// disagree about a column that, by design, is not part of the model and
    /// never reaches the wire.
    pub fn table_columns(&self, table: &str) -> Vec<String> {
        match (&self.pg_schema, std::env::var("TAKOMO_TEST_PG")) {
            (Some(schema), Ok(url)) => {
                let schema = schema.clone();
                let table = table.to_string();
                std::thread::scope(|sc| {
                    sc.spawn(|| {
                        let mut c =
                            postgres::Client::connect(&url, postgres::NoTls).expect("connect pg");
                        c.query(
                            "SELECT column_name FROM information_schema.columns \
                             WHERE table_schema = $1 AND table_name = $2 \
                             ORDER BY ordinal_position",
                            &[&schema, &table],
                        )
                        .expect("information_schema")
                        .iter()
                        .map(|r| r.get::<_, String>(0))
                        .filter(|c| c != "seq")
                        .collect()
                    })
                    .join()
                    .expect("columns thread")
                })
            }
            _ => {
                let conn = rusqlite::Connection::open(self.db_path()).expect("open db");
                let mut stmt = conn
                    .prepare(&format!("PRAGMA table_info({table})"))
                    .expect("prepare table_info");
                let cols = stmt
                    .query_map([], |r| r.get::<_, String>(1))
                    .expect("query table_info")
                    .collect::<Result<Vec<_>, _>>()
                    .expect("read column names");
                cols
            }
        }
    }

    /// Read one TEXT value, on whichever backend is live. For the handful of
    /// tests that assert on a stored payload rather than on the API response.
    pub fn scalar_text(&self, sql: &str) -> String {
        match (&self.pg_schema, std::env::var("TAKOMO_TEST_PG")) {
            (Some(schema), Ok(url)) => {
                let schema = schema.clone();
                let sql = takomo::store::sql::pg_translate(sql);
                std::thread::scope(|sc| {
                    sc.spawn(|| {
                        let mut c =
                            postgres::Client::connect(&url, postgres::NoTls).expect("connect pg");
                        c.batch_execute(&format!("SET search_path TO {schema}"))
                            .expect("search_path");
                        c.query_one(sql.as_str(), &[])
                            .expect("scalar_text")
                            .get::<_, String>(0)
                    })
                    .join()
                    .expect("scalar thread")
                })
            }
            _ => {
                let conn = rusqlite::Connection::open(self.db_path()).expect("open db");
                conn.query_row(sql, [], |r| r.get(0)).expect("scalar_text")
            }
        }
    }

    /// Count rows in `table`, on whichever backend is live.
    pub fn count_rows(&self, table: &str) -> i64 {
        match (&self.pg_schema, std::env::var("TAKOMO_TEST_PG")) {
            (Some(schema), Ok(url)) => {
                let schema = schema.clone();
                let sql = format!("SELECT COUNT(*) FROM {table}");
                std::thread::scope(|sc| {
                    sc.spawn(|| {
                        let mut c =
                            postgres::Client::connect(&url, postgres::NoTls).expect("connect pg");
                        c.batch_execute(&format!("SET search_path TO {schema}"))
                            .expect("search_path");
                        c.query_one(sql.as_str(), &[]).expect("count").get::<_, i64>(0)
                    })
                    .join()
                    .expect("count thread")
                })
            }
            _ => {
                let conn = rusqlite::Connection::open(self.db_path()).expect("open db");
                conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                    .expect("count")
            }
        }
    }

    /// Run a statement straight at the database, bypassing `Store`.
    ///
    /// These are the harness's escape hatches: a parent cycle, a backdated
    /// schedule slot, an uninterpretable share `kind` — states the API
    /// deliberately will not produce, which is exactly why a test has to reach
    /// them another way. They used to open a raw rusqlite connection to the
    /// SQLite file; against Postgres there is no file, so they route through
    /// here and the SQL is translated by the SAME shim the store uses, rather
    /// than a second hand-maintained copy of the dialect rules.
    pub fn force_sql(&self, sql: &str, params: &[SqlValue]) -> usize {
        self.force_sql_inner(sql, params, false)
    }

    /// As [`TestApp::force_sql`], with foreign keys disabled for the statement.
    /// Only `force_parent` needs it, to build a cycle SQLite would otherwise
    /// refuse.
    pub fn force_sql_no_fk(&self, sql: &str, params: &[SqlValue]) -> usize {
        self.force_sql_inner(sql, params, true)
    }

    /// Run one statement many times over a SINGLE connection.
    ///
    /// `force_sql` opens a connection per call, which is fine for the one-row
    /// escape hatches and catastrophic for `seed_bulk_tickets`: 5000 rows became
    /// 5000 Postgres connect/disconnect cycles and the export test stopped
    /// finishing. One connection for the batch, as the SQLite arm always did
    /// with its single transaction.
    pub fn force_sql_many(&self, sql: &str, rows: &[Vec<SqlValue>]) {
        match (&self.pg_schema, std::env::var("TAKOMO_TEST_PG")) {
            (Some(schema), Ok(url)) => {
                let translated = takomo::store::sql::pg_translate(sql);
                let schema = schema.clone();
                std::thread::scope(|sc| {
                    sc.spawn(|| {
                        let mut c =
                            postgres::Client::connect(&url, postgres::NoTls).expect("connect pg");
                        c.batch_execute(&format!("SET search_path TO {schema}"))
                            .expect("search_path");
                        let stmt = c.prepare(translated.as_str()).expect("prepare");
                        let mut tx = c.transaction().expect("begin");
                        for row in rows {
                            let bound: Vec<&(dyn postgres::types::ToSql + Sync)> = row
                                .iter()
                                .map(|v| v as &(dyn postgres::types::ToSql + Sync))
                                .collect();
                            tx.execute(&stmt, &bound).expect("force_sql_many");
                        }
                        tx.commit().expect("commit");
                    })
                    .join()
                    .expect("force_sql_many thread");
                });
            }
            _ => {
                let mut conn = rusqlite::Connection::open(self.db_path()).expect("open db");
                conn.busy_timeout(Duration::from_secs(10))
                    .expect("busy timeout");
                let tx = conn.transaction().expect("begin");
                for row in rows {
                    let bound: Vec<rusqlite::types::Value> = row
                        .iter()
                        .map(|v| match v {
                            SqlValue::Null => rusqlite::types::Value::Null,
                            SqlValue::Integer(i) => rusqlite::types::Value::Integer(*i),
                            SqlValue::Real(f) => rusqlite::types::Value::Real(*f),
                            SqlValue::Text(t) => rusqlite::types::Value::Text(t.clone()),
                            SqlValue::Blob(b) => rusqlite::types::Value::Blob(b.clone()),
                        })
                        .collect();
                    tx.execute(sql, rusqlite::params_from_iter(bound))
                        .expect("force_sql_many");
                }
                tx.commit().expect("commit");
            }
        }
    }

    fn force_sql_inner(&self, sql: &str, params: &[SqlValue], no_fk: bool) -> usize {
        match (&self.pg_schema, std::env::var("TAKOMO_TEST_PG")) {
            (Some(schema), Ok(url)) => {
                let translated = takomo::store::sql::pg_translate(sql);
                let schema = schema.clone();
                // Off-runtime for the same reason the store is: the sync driver
                // panics if its block_on runs inside a tokio runtime.
                std::thread::scope(|sc| {
                    sc.spawn(|| {
                        let mut c =
                            postgres::Client::connect(&url, postgres::NoTls).expect("connect pg");
                        c.batch_execute(&format!("SET search_path TO {schema}"))
                            .expect("search_path");
                        if no_fk {
                            // Session-scoped equivalent of SQLite's pragma.
                            c.batch_execute("SET session_replication_role = replica")
                                .expect("fk off");
                        }
                        let bound: Vec<&(dyn postgres::types::ToSql + Sync)> = params
                            .iter()
                            .map(|v| v as &(dyn postgres::types::ToSql + Sync))
                            .collect();
                        c.execute(translated.as_str(), &bound).expect("force_sql") as usize
                    })
                    .join()
                    .expect("force_sql thread")
                })
            }
            _ => {
                let conn = rusqlite::Connection::open(self.db_path()).expect("open db");
                conn.busy_timeout(Duration::from_secs(10))
                    .expect("busy timeout");
                if no_fk {
                    conn.pragma_update(None, "foreign_keys", "OFF")
                        .expect("foreign_keys off");
                }
                let bound: Vec<rusqlite::types::Value> = params
                    .iter()
                    .map(|v| match v {
                        SqlValue::Null => rusqlite::types::Value::Null,
                        SqlValue::Integer(i) => rusqlite::types::Value::Integer(*i),
                        SqlValue::Real(f) => rusqlite::types::Value::Real(*f),
                        SqlValue::Text(t) => rusqlite::types::Value::Text(t.clone()),
                        SqlValue::Blob(b) => rusqlite::types::Value::Blob(b.clone()),
                    })
                    .collect();
                conn.execute(sql, rusqlite::params_from_iter(bound))
                    .expect("force_sql")
            }
        }
    }

    pub fn open_store(&self) -> Store {
        match (&self.pg_schema, std::env::var("TAKOMO_TEST_PG")) {
            (Some(schema), Ok(url)) => Store::connect_pg_in(&url, schema).unwrap(),
            _ => Store::open(self.db_path()).unwrap(),
        }
    }

    /// Mint an extra token straight in the server's DB — the CLI's root of
    /// trust, and the only way to get scopes (`autoland`, `expert:*`) or a
    /// project allowlist that the standard four tokens do not have.
    pub fn mint(&self, actor: &str, scope_list: &[&str], projects: Option<&[&str]>) -> String {
        self.mint_limited(actor, scope_list, projects, 10_000)
    }

    /// [`TestApp::mint`] with an explicit per-token write rate limit.
    pub fn mint_limited(
        &self,
        actor: &str,
        scope_list: &[&str],
        projects: Option<&[&str]>,
        rate_limit: i64,
    ) -> String {
        let projects: Option<Vec<String>> = projects.map(scope_vec);
        let store = self.open_store();
        let (_, plaintext) = store
            .create_token(
                actor,
                &scope_vec(scope_list),
                projects.as_deref(),
                rate_limit,
                None,
            )
            .expect("mint token");
        plaintext
    }

    /// Bulk-insert `n` extra tickets straight into the running server's DB in a
    /// single transaction. The only practical way to grow the table until an
    /// unfiltered scan (`GET /v1/export`) takes measurable time — the same
    /// number of tickets over HTTP would be a minute of round-trips. Rows are
    /// minimal but valid, so the export path reads them back through the normal
    /// ticket mapping. Ids are `bulk-NNNNNN`, well clear of the generated ones.
    pub fn seed_bulk_tickets(&self, n: usize) {
        let base = 1_700_000_000_000i64;
        let rows: Vec<Vec<SqlValue>> = (0..n)
            .map(|i| {
                vec![
                    v_text(format!("bulk-{i:06}")),
                    v_text(format!("Bulk ticket {i}")),
                    v_int(base + i as i64),
                ]
            })
            .collect();
        self.force_sql_many(
            "INSERT INTO tickets (id, project, type, title, state, priority, created_by, created_at, updated_at) \
             VALUES (?1, 'tp', 'task', ?2, 'brief', 'normal', 'test:bulk', ?3, ?3)",
            &rows,
        );
    }

    /// Write a raw `kind` into a `shares` row, bypassing the typed store path.
    /// The API can only ever store `project`/`subtree` (`ShareKind::as_str`), so
    /// this is the only way to produce the row a future code path — or a
    /// hand-edited database — could leave behind: one whose scope kind the store
    /// cannot interpret. Used to prove that read fails **closed**.
    pub fn force_share_kind(&self, share_id: &str, kind: &str) {
        let n = self.force_sql(
            "UPDATE shares SET kind = ?2 WHERE id = ?1",
            &[v_text(share_id), v_text(kind)],
        );
        assert_eq!(
            n, 1,
            "force_share_kind should touch exactly one row ({share_id})"
        );
    }

    /// Repoint `id`'s parent straight in the database, bypassing validation.
    pub fn force_parent(&self, id: &str, parent: &str) {
        // Deliberately bypasses the FK: the point is to build a parent CYCLE,
        // which no valid path can produce. SQLite needs foreign_keys OFF for
        // that; Postgres has no self-referential violation here because the
        // target row does exist — only the cycle is nonsense.
        let n = self.force_sql_no_fk(
            "UPDATE tickets SET parent = ?2 WHERE id = ?1",
            &[v_text(id), v_text(parent)],
        );
        assert_eq!(n, 1, "force_parent should touch exactly one row ({id})");
    }

    /// Backdate a case's agent verdict so time-based expiry can be tested without
    /// sleeping for a month. Writes straight to the file, the same trick
    /// `force_parent` uses.
    pub fn backdate_case_verdict(&self, case: &str, millis_ago: i64) {
        let when = chrono::Utc::now().timestamp_millis() - millis_ago;
        let n = self.force_sql(
            "UPDATE cases SET agent_at = ?2 WHERE id = ?1",
            &[v_text(case), v_int(when)],
        );
        assert_eq!(n, 1, "backdate should touch exactly one row ({case})");
    }

    // --- schedules -----------------------------------------------------------

    /// Backdate a schedule's next slot so the sweep considers it due.
    ///
    /// The alternative is waiting for a real slot to arrive, and the finest
    /// cadence is daily — so without this every materialization test would be
    /// untestable rather than merely slow. Writes the column directly because the
    /// API deliberately has no way to set it: `next_slot` is server-owned.
    pub fn force_schedule_slot(&self, id: &str, slot_ms: i64) {
        let n = self.force_sql(
            "UPDATE schedules SET next_slot = ?2 WHERE id = ?1",
            &[v_text(id), v_int(slot_ms)],
        );
        assert_eq!(
            n, 1,
            "force_schedule_slot should touch exactly one row ({id})"
        );
    }

    /// Backdate a scheduled ticket's deadline, so "the clock ran out" is
    /// observable without waiting a week for it.
    pub fn force_ticket_expiry(&self, id: &str, expires_ms: i64) {
        let n = self.force_sql(
            "UPDATE tickets SET expires_at = ?2 WHERE id = ?1",
            &[v_text(id), v_int(expires_ms)],
        );
        assert_eq!(
            n, 1,
            "force_ticket_expiry should touch exactly one row ({id})"
        );
    }

    // --- tickets -------------------------------------------------------------

    pub async fn create_ticket(&self, title: &str) -> String {
        let (status, body) = self
            .post(
                &self.admin,
                "/v1/tickets",
                json!({ "project": "tp", "title": title }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "create failed: {body}");
        body["id"].as_str().expect("ticket id").to_string()
    }

    /// Create a ticket with an explicit type and optional parent (admin token).
    pub async fn create_typed(&self, title: &str, ty: &str, parent: Option<&str>) -> String {
        let mut body = json!({ "project": "tp", "title": title, "type": ty });
        if let Some(p) = parent {
            body["parent"] = json!(p);
        }
        let (status, b) = self.post(&self.admin, "/v1/tickets", body).await;
        assert_eq!(status, StatusCode::CREATED, "create_typed failed: {b}");
        b["id"].as_str().expect("ticket id").to_string()
    }

    pub async fn transition(&self, token: &str, id: &str, to: &str) -> (StatusCode, Value) {
        self.post(
            token,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": to }),
        )
        .await
    }

    /// brief -> spec -> ready (human approval path).
    pub async fn to_ready(&self, id: &str) {
        let (s1, b1) = self.transition(&self.human, id, "spec").await;
        assert_eq!(s1, StatusCode::OK, "brief->spec failed: {b1}");
        let (s2, b2) = self.transition(&self.human, id, "ready").await;
        assert_eq!(s2, StatusCode::OK, "spec->ready failed: {b2}");
    }

    /// Claim `id` as the default worker; asserts 200 and returns the lease fence.
    pub async fn claim(&self, id: &str) -> i64 {
        self.claim_as(&self.worker, id).await
    }

    /// Claim `id` as `token`; asserts 200 and returns the lease fence.
    pub async fn claim_as(&self, token: &str, id: &str) -> i64 {
        self.claim_ttl(token, id, None).await
    }

    /// Claim `id` as `token` with an explicit lease TTL; asserts 200 and returns
    /// the lease fence.
    pub async fn claim_ttl(&self, token: &str, id: &str, ttl_seconds: Option<i64>) -> i64 {
        let body = match ttl_seconds {
            Some(ttl) => json!({ "ttl_seconds": ttl }),
            None => json!({}),
        };
        let (s, lease) = self
            .post(token, &format!("/v1/tickets/{id}/claim"), body)
            .await;
        assert_eq!(s, StatusCode::OK, "claim failed: {lease}");
        lease["fence"].as_i64().expect("lease fence")
    }

    /// Drive a leaf ticket all the way to `done`: ready -> claim -> implementing
    /// -> review -> done (the human gate auto-releases the worker's claim).
    pub async fn drive_to_done(&self, id: &str) {
        let fence = self.to_implementing(id).await;
        let (s, b) = self
            .post(
                &self.worker,
                &format!("/v1/tickets/{id}/transition"),
                json!({ "to": "review", "fence": fence }),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "->review failed: {b}");
        let (s, b) = self.transition(&self.human, id, "done").await;
        assert_eq!(s, StatusCode::OK, "->done failed: {b}");
    }

    /// Drive a leaf ticket to `implementing` and return the worker's fence, so
    /// question tests can park an in-progress ticket.
    pub async fn to_implementing(&self, id: &str) -> i64 {
        self.to_ready(id).await;
        let fence = self.claim(id).await;
        let (s, b) = self
            .post(
                &self.worker,
                &format!("/v1/tickets/{id}/transition"),
                json!({ "to": "implementing", "fence": fence }),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "->implementing failed: {b}");
        fence
    }

    // --- questions -----------------------------------------------------------

    /// Ask question `q` as `token`; asserts 201 and returns (question id, ask
    /// response). The `ticket`/`fence`/`mode` keys stay in `q` at the call site
    /// because they are exactly what each test is varying.
    ///
    /// Only for the asks that are expected to succeed — a test checking a
    /// refusal posts `/v1/questions` through [`TestApp::post`] and asserts the
    /// status itself.
    pub async fn ask(&self, token: &str, q: Value) -> (String, Value) {
        let (s, body) = self.post(token, "/v1/questions", q).await;
        assert_eq!(s, StatusCode::CREATED, "ask failed: {body}");
        let qid = body["question"]["id"]
            .as_str()
            .expect("question id")
            .to_string();
        (qid, body)
    }

    /// Answer question `qid` as `token`. The status is returned rather than
    /// asserted, because roughly half the call sites are checking a refusal.
    pub async fn answer(&self, token: &str, qid: &str, answer: Value) -> (StatusCode, Value) {
        self.post(
            token,
            &format!("/v1/questions/{qid}/answer"),
            json!({ "answer": answer }),
        )
        .await
    }

    // --- projects ------------------------------------------------------------

    /// Create project `id` running `workflow` instead of `factory-default`.
    /// The standard four tokens carry no project allowlist, so they reach it.
    pub async fn create_project_with(&self, id: &str, workflow: Value) {
        let (s, body) = self
            .post(
                &self.admin,
                "/v1/projects",
                json!({ "id": id, "name": id, "workflow": workflow }),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "create project {id} failed: {body}");
    }

    /// A ticket in project `project` — the `create_ticket` family is `tp`-only.
    pub async fn create_ticket_in(&self, project: &str, title: &str) -> String {
        let (s, body) = self
            .post(
                &self.admin,
                "/v1/tickets",
                json!({ "project": project, "title": title }),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "create failed: {body}");
        body["id"].as_str().expect("ticket id").to_string()
    }

    /// The row for project `id` as `token` sees it in `GET /v1/projects`.
    pub async fn project(&self, token: &str, id: &str) -> Value {
        let (s, list) = self.get(token, "/v1/projects").await;
        assert_eq!(s, StatusCode::OK, "project list failed: {list}");
        list.as_array()
            .expect("project list is an array")
            .iter()
            .find(|p| p["id"] == id)
            .unwrap_or_else(|| panic!("project {id} not in list: {list}"))
            .clone()
    }
}

/// The shipped `simple` workflow (`workflows/simple.yaml`) as the JSON the
/// project endpoints take. Read from the file rather than copied inline, so a
/// test can never end up asserting against a stale duplicate of the workflow
/// `takomo init` actually applies.
pub fn simple_workflow() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("workflows/simple.yaml");
    let yaml =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_norway::from_str(&yaml).expect("workflows/simple.yaml is a workflow document")
}
