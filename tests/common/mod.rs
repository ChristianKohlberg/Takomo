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
}

fn scope_vec(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

impl TestApp {
    pub async fn spawn() -> TestApp {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = Store::open(tmp.path().join("test.db")).expect("open store");
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

        let state = AppState::new(store);
        spawn_sweeper(state.clone(), Duration::from_millis(250));
        let router = build_router(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        TestApp {
            base: format!("http://{addr}"),
            admin,
            human,
            worker,
            worker2,
            client: reqwest::Client::new(),
            tmp,
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
    pub fn open_store(&self) -> Store {
        Store::open(self.db_path()).unwrap()
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
        let mut conn = rusqlite::Connection::open(self.db_path()).expect("open db");
        conn.busy_timeout(Duration::from_secs(10))
            .expect("busy timeout");
        let tx = conn.transaction().expect("begin");
        let base = 1_700_000_000_000i64;
        for i in 0..n {
            tx.execute(
                "INSERT INTO tickets (id, project, type, title, state, priority, created_by, created_at, updated_at) \
                 VALUES (?1, 'tp', 'task', ?2, 'brief', 'normal', 'test:bulk', ?3, ?3)",
                rusqlite::params![
                    format!("bulk-{i:06}"),
                    format!("Bulk ticket {i}"),
                    base + i as i64
                ],
            )
            .expect("insert bulk ticket");
        }
        tx.commit().expect("commit bulk tickets");
    }

    /// Write a raw `kind` into a `shares` row, bypassing the typed store path.
    /// The API can only ever store `project`/`subtree` (`ShareKind::as_str`), so
    /// this is the only way to produce the row a future code path — or a
    /// hand-edited database — could leave behind: one whose scope kind the store
    /// cannot interpret. Used to prove that read fails **closed**.
    pub fn force_share_kind(&self, share_id: &str, kind: &str) {
        let conn = rusqlite::Connection::open(self.db_path()).expect("open db");
        conn.busy_timeout(Duration::from_secs(5))
            .expect("busy timeout");
        let n = conn
            .execute(
                "UPDATE shares SET kind = ?2 WHERE id = ?1",
                rusqlite::params![share_id, kind],
            )
            .expect("force share kind");
        assert_eq!(
            n, 1,
            "force_share_kind should touch exactly one row ({share_id})"
        );
    }

    /// Repoint `id`'s parent straight in the database, bypassing validation.
    pub fn force_parent(&self, id: &str, parent: &str) {
        let conn = rusqlite::Connection::open(self.db_path()).expect("open db");
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .expect("busy timeout");
        conn.pragma_update(None, "foreign_keys", "OFF")
            .expect("foreign_keys off");
        let n = conn
            .execute(
                "UPDATE tickets SET parent = ?2 WHERE id = ?1",
                rusqlite::params![id, parent],
            )
            .expect("force parent");
        assert_eq!(n, 1, "force_parent should touch exactly one row ({id})");
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
