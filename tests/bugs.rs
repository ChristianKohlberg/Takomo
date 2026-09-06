mod common;
use common::TestApp;
use reqwest::StatusCode;
use serde_json::{json, Value};
async fn bug(app: &TestApp, title: &str) -> String {
    let(s,v)=app.post(&app.worker,"/v1/tickets",json!({"project":"tp","type":"bug","title":title,"body":"Actual: error. Expected: success."})).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().unwrap().to_owned()
}
async fn configure(app: &TestApp) {
    let (s, v) = app
        .put(
            &app.admin,
            "/v1/projects/tp/bug-research-config",
            json!({"repository":"tp","revision":"HEAD","enabled":true}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
}
async fn start(app: &TestApp, id: &str, key: &str) -> Value {
    let (s, v) = app
        .post(
            &app.worker,
            &format!("/v1/bugs/{id}/research"),
            json!({"request_id":key}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["jobs"][0].clone()
}
async fn claim(app: &TestApp, token: &str, service: &str) -> Value {
    let (s, v) = app
        .post(token, "/v1/agent-jobs/claim", json!({"service_id":service}))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["job"].clone()
}
#[tokio::test]
async fn bugs_are_tickets_research_is_explicit_and_review_preserves_workflow() {
    let app = TestApp::spawn_without_sweeper().await;
    let id = bug(&app, "Receipt error").await;
    let path = format!("/v1/bugs/{id}");
    let before = app.get(&app.worker, &format!("/v1/tickets/{id}")).await.1;
    let (s, v) = app
        .get(
            &app.worker,
            "/v1/bugs?project=tp&q=Receipt&view=needs_triage",
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["total"], 1);
    assert_eq!(v["items"][0]["severity"], "unknown");
    assert!(v["items"][0]["latest_job"].is_null());
    assert_eq!(
        app.post(
            &app.worker,
            &format!("{path}/research"),
            json!({"request_id":"disabled"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    configure(&app).await;
    let run = start(&app, &id, "first").await;
    assert_eq!(run["status"], "queued");
    assert_eq!(start(&app, &id, "double-click").await["id"], run["id"]);
    let runner = app.mint("agent:research", &["agent:run"], Some(&["tp"]));
    let claimed = claim(&app, &runner, "local").await;
    assert_eq!(claimed["kind"], "bug_research");
    assert_eq!(claimed["ticket_id"], id);
    assert_eq!(claimed["repository_ref"]["repository"], "tp");
    let snapshot: Value = serde_json::from_str(claimed["snapshot"].as_str().unwrap()).unwrap();
    assert_eq!(snapshot["body"], before["body"]);
    let endpoint = format!("/v1/agent-jobs/{}", run["id"].as_str().unwrap());
    let hb = json!({"service_id":"local","attempt_id":claimed["attempt_id"],"thread_id":"thread","turn_id":"turn"});
    assert_eq!(
        app.post(
            &app.worker,
            &format!("{endpoint}/steer"),
            json!({"request_id":"steer","message":"Inspect the validation branch"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (s, v) = app
        .post(&runner, &format!("{endpoint}/heartbeat"), hb)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["steering"][0]["message"], "Inspect the validation branch");
    let result = json!({"service_id":"local","attempt_id":claimed["attempt_id"],"thread_id":"thread","turn_id":"turn","status":"completed","message":"Suspected input validation; runtime reproduction is missing.","repository_revision":"abcdef123","evidence":{"runtime_reproduced":false,"inspected":[{"path":"src/main.rs","start_line":1}]}});
    let (s, v) = app
        .post(&runner, &format!("{endpoint}/result"), result.clone())
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        app.post(&runner, &format!("{endpoint}/result"), result.clone())
            .await
            .0,
        StatusCode::OK
    );
    let (s, v) = app.get(&app.worker, &path).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["triage"], "ready_for_review");
    assert_eq!(v["latest_job"]["repository_revision"], "abcdef123");
    assert!(v["latest_job"]["snapshot"].is_string(), "{v}");
    assert_eq!(v["latest_job"]["response"], result["message"]);
    let (s, listed) = app.get(&app.worker, "/v1/bugs").await;
    assert_eq!(s, StatusCode::OK, "{listed}");
    let light = &listed["items"][0]["latest_job"];
    assert_eq!(light["id"], run["id"]);
    assert_eq!(light["kind"], "bug_research");
    assert_eq!(light["evidence"]["inspected"][0]["path"], "src/main.rs");
    assert!(
        light.get("prompt").is_none()
            && light.get("snapshot").is_none()
            && light.get("response").is_none(),
        "{light}"
    );
    let (s, jobs) = app.get(&app.worker, "/v1/agent-jobs?project=tp").await;
    assert_eq!(s, StatusCode::OK, "{jobs}");
    assert_eq!(jobs["counts"]["cancelled"], 0, "{jobs}");
    assert_eq!(jobs["counts"]["completed"], 1, "{jobs}");
    let item = &jobs["items"][0];
    assert_eq!(item["id"], run["id"]);
    assert_eq!(item["ticket_id"], id);
    assert_eq!(item["section_title"], before["title"]);
    assert_eq!(item["repository_revision"], "abcdef123");
    assert_eq!(
        item["steering"][0]["message"],
        "Inspect the validation branch"
    );
    assert!(
        item.get("prompt").is_none()
            && item.get("snapshot").is_none()
            && item.get("response").is_none(),
        "{item}"
    );
    let (s, one) = app.get(&app.worker, &endpoint).await;
    assert_eq!(s, StatusCode::OK, "{one}");
    assert_eq!(one["job"]["response"], result["message"]);
    assert_eq!(one["job"]["section_title"], before["title"]);
    assert_eq!(one["job"]["snapshot"], claimed["snapshot"]);
    let (s, history) = app.get(&app.worker, &format!("{path}/research")).await;
    assert_eq!(s, StatusCode::OK, "{history}");
    assert_eq!(history["jobs"][0]["response"], result["message"]);
    assert_eq!(history["jobs"][0]["snapshot"], claimed["snapshot"]);
    assert_eq!(v["ticket"]["state"], before["state"]);
    assert_eq!(v["ticket"]["priority"], before["priority"]);
    assert_eq!(v["ticket"]["claim"], before["claim"]);
    let (s, v) = app
        .patch(
            &app.human,
            &path,
            json!({"triage":"confirmed","severity":"high","note":"Reviewed evidence"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["severity"], "high");
    assert_eq!(v["ticket"]["version"], before["version"]);
    let (s, v) = app
        .patch(&app.human, &path, json!({"severity":"medium"}))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["note"], "Reviewed evidence");
    let (s, v) = app.patch(&app.human, &path, json!({"note":null})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["note"], "Reviewed evidence");
    let (s, v) = app.patch(&app.human, &path, json!({"note":"  "})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["note"].is_null(), "{v}");
    assert_eq!(v["triage"], "confirmed");
    let (s, v) = app.get(&app.worker, &path).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["note"].is_null(), "{v}");
    let (s, v) = app
        .patch(&app.human, &path, json!({"note":"x".repeat(8001)}))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}
#[tokio::test]
async fn bug_research_authorization_cancellation_expiry_and_project_capacity() {
    let app = TestApp::spawn_without_sweeper().await;
    configure(&app).await;
    let outsider = app.mint(
        "agent:outside",
        &["read", "write", "agent:run"],
        Some(&["elsewhere"]),
    );
    let reader = app.mint("human:reader", &["read"], Some(&["tp"]));
    let runner = app.mint("agent:research", &["agent:run"], Some(&["tp"]));
    let mut runs = Vec::new();
    for n in 0..3 {
        let id = bug(&app, &format!("Bug {n}")).await;
        runs.push((id.clone(), start(&app, &id, &format!("run-{n}")).await));
    }
    let id = &runs[0].0;
    let endpoint = format!("/v1/agent-jobs/{}", runs[0].1["id"].as_str().unwrap());
    assert_eq!(
        app.get(&outsider, &format!("/v1/bugs/{id}")).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(app.get(&outsider, "/v1/bugs").await.1["total"], 0);
    assert_eq!(
        app.post(&reader, &format!("{endpoint}/cancel"), json!({}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.put(
            &app.worker,
            "/v1/projects/tp/bug-research-config",
            json!({"repository":"tp","revision":"HEAD","enabled":true})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let one = claim(&app, &runner, "one").await;
    let two = claim(&app, &runner, "two").await;
    assert!(one.is_object() && two.is_object());
    assert!(claim(&app, &runner, "three").await.is_null());
    assert_eq!(
        app.post(&app.worker, &format!("{endpoint}/cancel"), json!({}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        app.get(&app.worker, &endpoint).await.1["job"]["status"],
        "cancelled"
    );
    let (s, v) = app
        .post(
            &runner,
            &format!("{endpoint}/heartbeat"),
            json!({"service_id":"one","attempt_id":one["attempt_id"]}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["cancel_requested"], true);
    assert!(claim(&app, &runner, "three").await.is_object());
    assert_eq!(app.post(&runner,&format!("{endpoint}/result"),json!({"service_id":"one","attempt_id":one["attempt_id"],"status":"failed","error":"late"})).await.0,StatusCode::CONFLICT);
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    conn.execute(
        "UPDATE agent_jobs SET lease_expires_at=0 WHERE id=?1",
        [two["id"].as_str().unwrap()],
    )
    .unwrap();
    assert_eq!(app.open_store().sweep_expired_agent_jobs().unwrap(), 1);
    assert_eq!(
        app.get(
            &app.worker,
            &format!("/v1/agent-jobs/{}", two["id"].as_str().unwrap())
        )
        .await
        .1["job"]["status"],
        "failed"
    );
    let again = start(&app, id, "retry").await;
    assert_ne!(again["id"], one["id"]);
    assert_eq!(
        app.get(&app.worker, &format!("/v1/bugs/{id}/research"))
            .await
            .1["total"],
        2
    );
}
#[tokio::test]
async fn bug_list_filters_duplicate_validation_and_archival() {
    let app = TestApp::spawn_without_sweeper().await;
    let first = bug(&app, "Receipt failure").await;
    let second = bug(&app, "Other error").await;
    let p = format!("/v1/bugs/{first}");
    assert_eq!(
        app.patch(&app.worker, &p, json!({"triage":"duplicate"}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.patch(
            &app.worker,
            &p,
            json!({"triage":"duplicate","duplicate_of":first})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.patch(
            &app.worker,
            &p,
            json!({"triage":"duplicate","duplicate_of":second,"severity":"critical"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (s, v) = app
        .get(&app.worker, "/v1/bugs?severity=critical&limit=1&offset=0")
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["total"], 1);
    assert_eq!(v["items"][0]["ticket"]["id"], first);
    let (s, v) = app.get(&app.worker, "/v1/bugs?limit=1").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["total"], 2);
    assert!(v["note"].is_string());
    assert_eq!(
        app.get(&app.worker, "/v1/bugs?q=Receipt").await.1["total"],
        1
    );
    let (s, v) = app
        .get(
            &app.worker,
            "/v1/bugs?view=ready_for_review&triage=confirmed",
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["code"], "validation.bug", "{v}");
    assert_eq!(
        app.get(
            &app.worker,
            "/v1/bugs?view=needs_triage&triage=needs_triage"
        )
        .await
        .1["total"],
        1
    );
    let (s, v) = app
        .get(&app.worker, "/v1/bugs?view=all&triage=duplicate")
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["total"], 1);
    assert_eq!(v["items"][0]["ticket"]["id"], first);
    assert_eq!(
        app.get(&app.worker, "/v1/bugs?assignee=none").await.1["total"],
        2
    );
    app.to_ready(&second).await;
    app.claim(&second).await;
    let (s, v) = app.get(&app.worker, "/v1/bugs?assignee=none").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["total"], 1);
    assert_eq!(v["items"][0]["ticket"]["id"], first);
    let (s, v) = app.get(&app.worker, "/v1/bugs?claimed_by=agent:w1").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["total"], 1);
    assert_eq!(v["items"][0]["ticket"]["id"], second);
    configure(&app).await;
    start(&app, &first, "before-archive").await;
    assert_eq!(
        app.post(
            &app.admin,
            &format!("/v1/tickets/{first}/archive"),
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(app.open_store().sweep_expired_agent_jobs().unwrap(), 1);
    assert_eq!(app.get(&app.worker, "/v1/bugs").await.1["total"], 1);
    assert_eq!(
        app.get(&app.worker, "/v1/bugs?view=all").await.1["total"],
        2
    );
    assert_eq!(
        app.post(
            &app.worker,
            &format!("{p}/research"),
            json!({"request_id":"archived"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}
#[tokio::test]
async fn legacy_conversation_migration_preserves_jobs_and_foreign_keys() {
    let app = TestApp::spawn_without_sweeper().await;
    let (s, v) = app
        .post(
            &app.admin,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Legacy"}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let map = v["mindmap"]["id"].as_str().unwrap();
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
    conn.execute_batch("BEGIN; CREATE TABLE agent_conversations_old(id TEXT PRIMARY KEY,mindmap TEXT NOT NULL REFERENCES mindmaps(id) ON DELETE CASCADE,node TEXT NOT NULL,project TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,service_id TEXT,thread_id TEXT,created_at INTEGER NOT NULL,UNIQUE(mindmap,node)); DROP TABLE agent_conversations; ALTER TABLE agent_conversations_old RENAME TO agent_conversations; COMMIT;").unwrap();
    conn.execute("INSERT INTO agent_conversations(id,mindmap,node,project,created_at) VALUES('old',?1,'node','tp',1)",[map]).unwrap();
    conn.execute_batch("INSERT INTO agent_jobs(id,conversation_id,requested_by,request_id,prompt,snapshot,source_revision,status,created_at) VALUES('old-job','old','human:one','once','hello','# Legacy','rev','queued',1); INSERT INTO agent_messages(id,conversation_id,job_id,role,body,created_at) VALUES('old-message','old','old-job','user','hello',1);").unwrap();
    drop(conn);
    let store = app.open_store();
    let old = store.agent_conversation(map, "node").unwrap();
    assert_eq!(old["jobs"][0]["id"], "old-job");
    assert_eq!(old["messages"][0]["body"], "hello");
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    let nullable: i64 = conn
        .query_row(
            "SELECT \"notnull\" FROM pragma_table_info('agent_conversations') WHERE name='mindmap'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(nullable, 0);
}
#[tokio::test]
async fn heartbeat_evidence_survives_cancellation_without_accepting_late_success() {
    let app = TestApp::spawn_without_sweeper().await;
    configure(&app).await;
    let id = bug(&app, "Evidence retention").await;
    let run = start(&app, &id, "evidence").await;
    let runner = app.mint("agent:research", &["agent:run"], Some(&["tp"]));
    let job = claim(&app, &runner, "local").await;
    let path = format!("/v1/agent-jobs/{}", run["id"].as_str().unwrap());
    let evidence = json!({"inspected":[{"path":"src/main.rs","start_line":1,"revision":"abc"}],"runtime_reproduced":false});
    let heartbeat = json!({"service_id":"local","attempt_id":job["attempt_id"],"repository_revision":"abc","evidence":evidence});
    let (status, value) = app
        .post(&runner, &format!("{path}/heartbeat"), heartbeat.clone())
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(
        app.get(&app.worker, &path).await.1["job"]["repository_revision"],
        "abc"
    );
    let mut changed = heartbeat;
    changed["repository_revision"] = json!("different");
    assert_eq!(
        app.post(&runner, &format!("{path}/heartbeat"), changed)
            .await
            .0,
        StatusCode::CONFLICT
    );
    app.post(&app.worker, &format!("{path}/cancel"), json!({}))
        .await;
    let result = json!({"service_id":"local","attempt_id":job["attempt_id"],"cancelled":true,"status":"failed","error":"Interrupted","repository_revision":"abc","evidence":evidence});
    let (status, value) = app.post(&runner, &format!("{path}/result"), result).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let detail = app.get(&app.worker, &path).await.1;
    assert_eq!(detail["job"]["status"], "cancelled");
    assert_eq!(detail["job"]["error"], "Cancelled by request");
    assert_eq!(detail["job"]["evidence"], evidence);
    assert!(detail["job"]["response"].is_null());
    let (status, listed) = app
        .get(&app.worker, "/v1/agent-jobs?status=cancelled")
        .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["total"], 1);
    assert_eq!(listed["counts"]["cancelled"], 1);
    assert_eq!(listed["counts"]["failed"], 0);
    assert_eq!(
        app.get(&app.worker, "/v1/agent-jobs?status=failed").await.1["total"],
        0
    );
    assert_eq!(
        app.get(&app.worker, "/v1/bugs?research_status=cancelled")
            .await
            .1["total"],
        1
    );
}

async fn beta(app: &TestApp) {
    let (s, v) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({"id":"beta","name":"Beta","workflow":{"name":"beta-wf","initial":"ready","states":[
                {"id":"ready","category":"todo","claimable":true},{"id":"done","category":"done","terminal":true}],
                "transitions":[{"from":"ready","to":"done"}]}}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
}
#[tokio::test]
async fn moving_a_bug_across_projects_keeps_research_and_triage_consistent() {
    let app = TestApp::spawn_without_sweeper().await;
    beta(&app).await;
    let moved = bug(&app, "Moves to beta").await;
    let stays = bug(&app, "Stays in tp").await;
    configure(&app).await;
    let run = start(&app, &moved, "before-move").await;
    let (s, v) = app
        .patch(
            &app.human,
            &format!("/v1/bugs/{stays}"),
            json!({"triage":"duplicate","duplicate_of":moved}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = app
        .post(
            &app.admin,
            "/v1/tickets/move",
            json!({"tickets":[moved],"to_project":"beta"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let beta_reader = app.mint("human:beta", &["read", "write", "human"], Some(&["beta"]));
    let (s, v) = app.get(&beta_reader, "/v1/bugs?project=beta").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["total"], 1);
    assert_eq!(v["items"][0]["ticket"]["id"], moved);
    assert_eq!(v["items"][0]["latest_job"]["id"], run["id"]);
    let (s, v) = app.get(&beta_reader, &format!("/v1/bugs/{moved}")).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["latest_job"]["project"], "beta");
    let (s, v) = app
        .get(&beta_reader, &format!("/v1/bugs/{moved}/research"))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["jobs"][0]["id"], run["id"]);
    let (s, v) = app.get(&app.worker, &format!("/v1/bugs/{stays}")).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["triage"], "needs_triage");
    assert!(v["duplicate_of"].is_null(), "{v}");
    let pair_a = bug(&app, "Pair A").await;
    let pair_b = bug(&app, "Pair B").await;
    let (s, v) = app
        .patch(
            &app.human,
            &format!("/v1/bugs/{pair_a}"),
            json!({"triage":"duplicate","duplicate_of":pair_b}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = app
        .post(
            &app.admin,
            "/v1/tickets/move",
            json!({"tickets":[pair_a,pair_b],"to_project":"beta"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = app.get(&beta_reader, &format!("/v1/bugs/{pair_a}")).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["triage"], "duplicate");
    assert_eq!(v["duplicate_of"], pair_b);
    let (s, v) = app
        .patch(
            &beta_reader,
            &format!("/v1/bugs/{pair_a}"),
            json!({"severity":"low"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["duplicate_of"], pair_b);
    let tp_runner = app.mint("agent:tp", &["agent:run"], Some(&["tp"]));
    assert!(claim(&app, &tp_runner, "tp-worker").await.is_null());
    let beta_runner = app.mint("agent:beta", &["agent:run"], Some(&["beta"]));
    let claimed = claim(&app, &beta_runner, "beta-worker").await;
    assert_eq!(claimed["ticket_id"], moved, "{claimed}");
    let (s, v) = app
        .get(&beta_reader, "/v1/agent-jobs?project=beta&status=running")
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["total"], 1);
    assert_eq!(v["counts"]["cancelled"], 0);
    let (s, v) = app
        .patch(
            &app.human,
            &format!("/v1/bugs/{stays}"),
            json!({"triage":"duplicate","duplicate_of":pair_a}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    let late = bug(&app, "Late duplicate").await;
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    conn.execute(
        "INSERT INTO bug_triage(ticket,triage,severity,duplicate_of) VALUES(?1,'duplicate','unknown',?2)",
        [&late, &pair_a],
    )
    .unwrap();
    drop(conn);
    let (s, v) = app.delete(&app.admin, "/v1/projects/beta?force=true").await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let (s, v) = app.get(&app.worker, "/v1/bugs").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["total"], 2);
    let (s, v) = app.get(&app.worker, &format!("/v1/bugs/{late}")).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["triage"], "needs_triage");
    assert!(v["duplicate_of"].is_null(), "{v}");
    let (s, v) = app
        .patch(
            &app.human,
            &format!("/v1/bugs/{late}"),
            json!({"severity":"high"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["severity"], "high");
}
#[tokio::test]
async fn coalesced_request_ids_replay_to_the_run_they_joined() {
    let app = TestApp::spawn_without_sweeper().await;
    configure(&app).await;
    let id = bug(&app, "Coalesced").await;
    let first = start(&app, &id, "r1").await;
    let joined = app
        .post(
            &app.worker,
            &format!("/v1/bugs/{id}/research"),
            json!({"request_id":"r2"}),
        )
        .await;
    assert_eq!(joined.0, StatusCode::OK, "{}", joined.1);
    assert_eq!(joined.1["total"], 1);
    assert_eq!(joined.1["jobs"][0]["id"], first["id"]);
    let runner = app.mint("agent:research", &["agent:run"], Some(&["tp"]));
    let claimed = claim(&app, &runner, "local").await;
    assert_eq!(claimed["id"], first["id"]);
    let endpoint = format!("/v1/agent-jobs/{}", first["id"].as_str().unwrap());
    let (s, v) = app
        .post(
            &runner,
            &format!("{endpoint}/result"),
            json!({"service_id":"local","attempt_id":claimed["attempt_id"],"thread_id":"thread","turn_id":"turn","status":"completed","message":"Done.","repository_revision":"abc","evidence":{"runtime_reproduced":false,"inspected":[]}}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let replay = app
        .post(
            &app.worker,
            &format!("/v1/bugs/{id}/research"),
            json!({"request_id":"r2"}),
        )
        .await;
    assert_eq!(replay.0, StatusCode::OK, "{}", replay.1);
    assert_eq!(replay.1["total"], 1);
    assert_eq!(replay.1["jobs"][0]["id"], first["id"]);
    assert_eq!(replay.1["jobs"][0]["status"], "completed");
    let (s, v) = app
        .post(
            &app.worker,
            &format!("/v1/bugs/{id}/research"),
            json!({"request_id":"r2","message":"Something else"}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    let (s, v) = app
        .post(
            &app.worker,
            &format!("/v1/bugs/{id}/research"),
            json!({"request_id":"r1"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["total"], 1);
    let fresh = start(&app, &id, "r3").await;
    assert_ne!(fresh["id"], first["id"]);
    assert_eq!(fresh["status"], "queued");
    let (s, v) = app
        .get(&app.worker, &format!("/v1/bugs/{id}/research"))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["total"], 2);
    let (s, v) = app
        .post(
            &app.worker,
            &format!("/v1/bugs/{id}/research"),
            json!({"request_id":"r2"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["total"], 2);
}
#[tokio::test]
async fn deleting_a_duplicate_target_project_clears_the_pointer_after_schema_rebuild() {
    let app = TestApp::spawn_without_sweeper().await;
    beta(&app).await;
    let target = bug(&app, "Target").await;
    let pointer = bug(&app, "Pointer").await;
    let (s, v) = app
        .patch(
            &app.human,
            &format!("/v1/bugs/{pointer}"),
            json!({"triage":"duplicate","duplicate_of":target}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
    conn.execute_batch("BEGIN; CREATE TABLE bug_triage_old(ticket TEXT PRIMARY KEY REFERENCES tickets(id) ON DELETE CASCADE,triage TEXT NOT NULL DEFAULT 'needs_triage',severity TEXT NOT NULL DEFAULT 'unknown',duplicate_of TEXT REFERENCES tickets(id),note TEXT,updated_by TEXT,updated_at INTEGER); INSERT INTO bug_triage_old SELECT ticket,triage,severity,duplicate_of,note,updated_by,updated_at FROM bug_triage; DROP TABLE bug_triage; ALTER TABLE bug_triage_old RENAME TO bug_triage; COMMIT;").unwrap();
    conn.execute(
        "UPDATE tickets SET project='beta', state='ready' WHERE id=?1",
        [&target],
    )
    .unwrap();
    drop(conn);
    drop(app.open_store());
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    let violations: i64 = conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0);
    drop(conn);
    let (s, v) = app.delete(&app.admin, "/v1/projects/beta").await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let (s, v) = app.get(&app.worker, &format!("/v1/bugs/{pointer}")).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["duplicate_of"].is_null(), "{v}");
    assert_eq!(v["triage"], "needs_triage");
    let (s, v) = app
        .patch(
            &app.human,
            &format!("/v1/bugs/{pointer}"),
            json!({"severity":"high"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["severity"], "high");
    assert_eq!(v["triage"], "needs_triage");
    assert_eq!(
        app.get(&app.worker, &format!("/v1/bugs/{target}")).await.0,
        StatusCode::NOT_FOUND
    );
}
