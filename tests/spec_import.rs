mod common;
use common::TestApp;
use reqwest::StatusCode;
use serde_json::{json, Value};
use yrs::{updates::decoder::Decode, Doc, Map, Transact, Update};

async fn fixture(app: &TestApp) -> String {
    let (status, body) = app
        .post(
            &app.admin,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Import review"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["mindmap"]["id"].as_str().unwrap().into()
}
fn draft() -> Value {
    json!({"request_id":"preview-1","revision":"a".repeat(40),"scope":{"include":["src"],"exclude":["src/private"]},
        "draft":{"title":"Checkout","summary":"A scoped, as-built draft.","gaps":["Payments outside src were not inspected."],
            "sections":[{"key":"checkout","parent":null,"title":"Checkout rules","notes":"Orders require an item.","sources":[{"path":"src/order.rs","start_line":1,"end_line":3}]},
                {"key":"errors","parent":"checkout","title":"Rejected orders","notes":"An empty order is refused.","sources":[{"path":"src/order.rs","start_line":4,"end_line":5}]}]}})
}
#[tokio::test]
async fn fixture_app_server_to_document_smoke() {
    let app = TestApp::spawn().await;
    let map = fixture(&app).await;
    let output = app.tmp.path().join("simulated-draft.json");
    let mut child = std::process::Command::new("node");
    child
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["services/agent/test/import-fixture-smoke.mjs", "--out"])
        .arg(&output)
        .args(["--mindmap", &map])
        .env("TAKOMO_URL", &app.base)
        .env("TAKOMO_IMPORT_TOKEN", &app.human)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let result = tokio::task::spawn_blocking(move || {
        let mut process = child
            .spawn()
            .expect("Node 22 is required for the fixture smoke");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while process.try_wait().unwrap().is_none() {
            if std::time::Instant::now() >= deadline {
                let _ = process.kill();
                let _ = process.wait();
                panic!("fixture smoke deadline");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        process.wait_with_output().unwrap()
    })
    .await
    .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let artifact: Value = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    assert_eq!(artifact["status"], "ready");
    assert_eq!(artifact["manifest"]["counts"]["selected_files"], 1);
    let (_, view) = app.get(&app.human, &format!("/v1/mindmaps/{map}")).await;
    let nodes = view["nodes"].as_array().unwrap();
    assert_eq!(
        nodes.len(),
        4,
        "retry must not duplicate the root or three sections"
    );
    assert!(nodes.iter().all(|node| node["origin"] == "agent"));
    let quote = nodes
        .iter()
        .find(|node| node["title"] == "Order quotes")
        .unwrap();
    let pricing = nodes
        .iter()
        .find(|node| node["title"] == "Discounts and shipping")
        .unwrap();
    assert_eq!(pricing["parent"], quote["id"]);
    let (_, prose) = app
        .get(&app.human, &format!("/v1/mindmaps/{map}/prose"))
        .await;
    let text = prose["markdown"].as_str().unwrap();
    assert!(text.contains("simulated draft"));
    assert!(text.contains("examples/extraction-fixture/checkout.mjs:10–12"));
    assert!(text.contains("Cancellation"));
    assert!(!text.contains("outside.txt"));
}
#[tokio::test]
async fn imported_sections_are_real_unreviewed_document_and_mindmap_content() {
    let app = TestApp::spawn().await;
    let map = fixture(&app).await;
    let path = format!("/v1/mindmaps/{map}/codebase-import");
    let (status, result) = app.post(&app.human, &path, draft()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["reviewed"], false);
    // Rehydrate from a fresh store connection: the response cannot depend on
    // this server process retaining the live replica or a debounced update.
    let restored = Doc::new();
    for blob in app.open_store().load_collab_updates(&map).unwrap() {
        restored
            .transact_mut()
            .apply_update(Update::decode_v1(&blob).unwrap())
            .unwrap();
    }
    assert_eq!(
        takomo::store::mindmapdoc::snapshot(&restored, &map).2.len(),
        3
    );
    assert_eq!(
        restored
            .get_or_insert_map("spec_import_receipts")
            .len(&restored.transact()),
        1
    );
    let (_, view) = app.get(&app.human, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(view["nodes"].as_array().unwrap().len(), 3);
    let nodes = view["nodes"].as_array().unwrap();
    let parent = nodes
        .iter()
        .find(|n| n["id"] == result["sections"]["checkout"])
        .unwrap();
    let child = nodes
        .iter()
        .find(|n| n["id"] == result["sections"]["errors"])
        .unwrap();
    assert_eq!(child["parent"], parent["id"]);
    assert_eq!(parent["origin"], "agent");
    let (_, prose) = app
        .get(&app.human, &format!("/v1/mindmaps/{map}/prose"))
        .await;
    let text = prose["markdown"].as_str().unwrap();
    assert!(text.contains("Checkout rules") && text.contains("Orders require an item."));
    assert!(text.contains("src/order.rs:1–3"));
    let (status, retry) = app.post(&app.human, &path, draft()).await;
    assert_eq!(status, StatusCode::OK, "{retry}");
    assert_eq!(retry, result);
    let node = result["sections"]["checkout"].as_str().unwrap();
    assert_eq!(
        app.patch(
            &app.human,
            &format!("/v1/mindmaps/{map}/nodes/{node}"),
            json!({"notes":"Human correction"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(app.post(&app.human, &path, draft()).await.1, result);
    let (_, prose) = app
        .get(&app.human, &format!("/v1/mindmaps/{map}/prose"))
        .await;
    assert!(prose["markdown"]
        .as_str()
        .unwrap()
        .contains("Human correction"));
    let mut changed = draft();
    changed["draft"]["summary"] = json!("Different content");
    assert_eq!(
        app.post(&app.human, &path, changed).await.0,
        StatusCode::CONFLICT
    );
}
#[tokio::test]
async fn simultaneous_imports_deduplicate_and_nonempty_targets_are_preserved() {
    let app = TestApp::spawn().await;
    let map = fixture(&app).await;
    let path = format!("/v1/mindmaps/{map}/codebase-import");
    let (a, b) = tokio::join!(
        app.post(&app.human, &path, draft()),
        app.post(&app.human, &path, draft())
    );
    assert_eq!(a.0, StatusCode::OK, "{}", a.1);
    assert_eq!(a, b);
    let mut different = draft();
    different["request_id"] = json!("another-run");
    assert_eq!(
        app.post(&app.human, &path, different).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.get(&app.human, &format!("/v1/mindmaps/{map}")).await.1["total"],
        3
    );
}
#[tokio::test]
async fn invalid_scope_tree_and_permissions_never_mutate_target() {
    let app = TestApp::spawn().await;
    let map = fixture(&app).await;
    let path = format!("/v1/mindmaps/{map}/codebase-import");
    assert_eq!(
        app.post(&app.worker, &path, draft()).await.0,
        StatusCode::FORBIDDEN
    );
    for mode in ["scope", "parent", "size"] {
        let mut body = draft();
        match mode {
            "scope" => {
                body["draft"]["sections"][0]["sources"][0]["path"] = json!("src/private/key")
            }
            "parent" => body["draft"]["sections"][1]["parent"] = json!("missing"),
            _ => body["draft"]["sections"][1]["notes"] = json!("x".repeat(9000)),
        }
        assert_eq!(
            app.post(&app.human, &path, body).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            app.get(&app.human, &format!("/v1/mindmaps/{map}")).await.1["total"],
            0
        );
    }
}
#[tokio::test]
async fn persistence_failure_rolls_back_the_tree_and_receipt_then_retry_succeeds() {
    let app = TestApp::spawn().await;
    let map = fixture(&app).await;
    let path = format!("/v1/mindmaps/{map}/codebase-import");
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_import BEFORE INSERT ON crdt_updates BEGIN SELECT RAISE(ABORT, 'simulated disk failure'); END;").unwrap();
    assert_eq!(
        app.post(&app.human, &path, draft()).await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        app.get(&app.human, &format!("/v1/mindmaps/{map}")).await.1["total"],
        0
    );
    conn.execute_batch("DROP TRIGGER fail_import;").unwrap();
    assert_eq!(app.post(&app.human, &path, draft()).await.0, StatusCode::OK);
    assert_eq!(
        app.get(&app.human, &format!("/v1/mindmaps/{map}")).await.1["total"],
        3
    );
}

#[tokio::test]
async fn concurrent_reset_and_import_leave_the_live_and_durable_tree_in_agreement() {
    for _ in 0..3 {
        let app = TestApp::spawn().await;
        let map = fixture(&app).await;
        let reset_path = format!("/v1/mindmaps/{map}/reset");
        let import_path = format!("/v1/mindmaps/{map}/codebase-import");
        let (reset, import) = tokio::join!(
            app.post(&app.admin, &reset_path, json!({"confirm_id":map})),
            app.post(&app.human, &import_path, draft()),
        );
        assert_eq!(reset.0, StatusCode::OK, "{}", reset.1);
        assert_eq!(import.0, StatusCode::OK, "{}", import.1);
        let (_, view) = app.get(&app.human, &format!("/v1/mindmaps/{map}")).await;
        let count = view["nodes"].as_array().unwrap().len();
        assert!([0, 3].contains(&count));
        let restored = Doc::new();
        for blob in app.open_store().load_collab_updates(&map).unwrap() {
            restored
                .transact_mut()
                .apply_update(Update::decode_v1(&blob).unwrap())
                .unwrap();
        }
        assert_eq!(
            takomo::store::mindmapdoc::snapshot(&restored, &map).2.len(),
            count
        );
    }
}

#[tokio::test]
async fn retry_after_reset_or_deletion_refuses_instead_of_restoring_content() {
    let app = TestApp::spawn().await;
    let map = fixture(&app).await;
    let path = format!("/v1/mindmaps/{map}/codebase-import");
    let view = format!("/v1/mindmaps/{map}");
    let (status, result) = app.post(&app.human, &path, draft()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(
        app.post(
            &app.admin,
            &format!("/v1/mindmaps/{map}/reset"),
            json!({"confirm_id":map})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(app.get(&app.human, &view).await.1["total"], 0);
    let (status, body) = app.post(&app.human, &path, draft()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "conflict.spec_import_stale");
    assert_eq!(app.get(&app.human, &view).await.1["total"], 0);
    let mut fresh = draft();
    fresh["request_id"] = json!("preview-2");
    let (status, again) = app.post(&app.human, &path, fresh.clone()).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_ne!(again["root"], result["root"]);
    assert_eq!(app.get(&app.human, &view).await.1["total"], 3);
    assert_eq!(app.post(&app.human, &path, fresh.clone()).await.1, again);
    let root = again["root"].as_str().unwrap();
    assert_eq!(
        app.delete(&app.human, &format!("/v1/mindmaps/{map}/nodes/{root}"))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(app.get(&app.human, &view).await.1["total"], 0);
    let (status, body) = app.post(&app.human, &path, fresh).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "conflict.spec_import_stale");
    assert_eq!(app.get(&app.human, &view).await.1["total"], 0);
}
