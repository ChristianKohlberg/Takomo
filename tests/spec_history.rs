mod common;
use common::TestApp;
use reqwest::StatusCode;
use serde_json::{json, Value};
use yrs::{updates::decoder::Decode, Doc, ReadTxn, StateVector, Transact, Update};

async fn fixture(app: &TestApp) -> (String, String) {
    let (s, map) = app
        .post(
            &app.admin,
            "/v1/mindmaps",
            json!({"project":"tp","title":"History"}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{map}");
    let id = map["mindmap"]["id"].as_str().unwrap().to_owned();
    let (s, node) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{id}/nodes"),
            json!({"text":"Agreement","notes":"Old wording"}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{node}");
    (id, node["nodes"][0]["id"].as_str().unwrap().into())
}
async fn history(app: &TestApp, map: &str) -> Value {
    let (s, v) = app
        .get(&app.worker, &format!("/v1/mindmaps/{map}/versions"))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}
#[tokio::test]
async fn versions_keep_complete_content_after_edits_deletion_and_compaction() {
    let app = TestApp::spawn().await;
    let (map, node) = fixture(&app).await;
    let version = history(&app, &map).await["head"].as_i64().unwrap();
    let path = format!("/v1/mindmaps/{map}/versions/{version}");
    let (_, before) = app.get(&app.worker, &path).await;
    assert_eq!(before["nodes"][0]["notes"], "Old wording");
    let long = "Complete prose survives. ".repeat(500);
    let (s, b) = app
        .patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}"),
            json!({"title":"New agreement"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    // Rich-text collaboration accepts longer prose than the legacy REST notes field.
    let replica = Doc::new();
    let store = app.open_store();
    for blob in store.load_collab_updates(&map).unwrap() {
        replica
            .transact_mut()
            .apply_update(Update::decode_v1(&blob).unwrap())
            .unwrap();
    }
    let prose = takomo::store::mindmapdoc::read_section_prose(&replica, &node).unwrap();
    let delta = {
        let mut tx = replica.transact_mut();
        takomo::store::prose::set_plain_text(&mut tx, &prose, &long);
        tx.encode_update_v1()
    };
    store
        .append_collab_update(&map, &delta, "collaborator")
        .unwrap();
    let second = history(&app, &map).await["head"].as_i64().unwrap();
    let (_, saved) = app
        .get(
            &app.worker,
            &format!("/v1/mindmaps/{map}/versions/{second}"),
        )
        .await;
    assert_eq!(saved["nodes"][0]["notes"], long);
    assert!(saved["nodes"][0]["prose_xml"]
        .as_str()
        .unwrap()
        .contains("<paragraph>"));
    app.delete(&app.worker, &format!("/v1/mindmaps/{map}/nodes/{node}"))
        .await;
    let store = app.open_store();
    // Cross a materialization boundary, then compact the canonical log from its true state.
    let empty = Doc::new()
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let extra = Doc::new();
    let field = extra.get_or_insert_text("materialization-probe");
    for i in 0..70 {
        use yrs::Text;
        let blob = {
            let mut tx = extra.transact_mut();
            field.insert(&mut tx, i, "x");
            tx.encode_update_v1()
        };
        store.append_collab_update(&map, &blob, "test").unwrap();
    }
    let doc = Doc::new();
    for blob in store.load_collab_updates(&map).unwrap() {
        doc.transact_mut()
            .apply_update(Update::decode_v1(&blob).unwrap())
            .unwrap();
    }
    let state = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let count = store.load_collab_updates(&map).unwrap().len() as i64;
    let total = history(&app, &map).await["total"].clone();
    store
        .compact_collab(&map, &state, "test", 0, count)
        .unwrap();
    assert_eq!(history(&app, &map).await["total"], total);
    assert_eq!(app.get(&app.worker, &path).await.1, before);
    let response = app
        .client
        .get(format!("{}{path}/state", app.base))
        .bearer_auth(&app.worker)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );
    let recovered = Doc::new();
    recovered
        .transact_mut()
        .apply_update(Update::decode_v1(&response.bytes().await.unwrap()).unwrap())
        .unwrap();
    assert_eq!(
        takomo::store::mindmapdoc::snapshot(&recovered, &map).0[0]["notes"],
        "Old wording"
    );
    // Stable descending cursors do not repeat rows when a new save arrives.
    let (_, page) = app
        .get(&app.worker, &format!("/v1/mindmaps/{map}/versions?limit=2"))
        .await;
    let cursor = page["next_cursor"].as_i64().unwrap();
    store.append_collab_update(&map, &empty, "new").unwrap();
    let (_, older) = app
        .get(
            &app.worker,
            &format!("/v1/mindmaps/{map}/versions?limit=2&before={cursor}"),
        )
        .await;
    assert!(older["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v["version"].as_i64().unwrap() < cursor));
    let latest = history(&app, &map).await["head"].as_i64().unwrap();
    assert!(app
        .get(
            &app.worker,
            &format!("/v1/mindmaps/{map}/versions/{latest}")
        )
        .await
        .1["nodes"]
        .as_array()
        .unwrap()
        .is_empty());
}
#[tokio::test]
async fn checkpoint_is_immutable_cas_guarded_and_project_scoped() {
    let app = TestApp::spawn().await;
    let (map, node) = fixture(&app).await;
    let h = history(&app, &map).await;
    let request = json!({"name":"Agreed scope","expected_version":h["head"]});
    let route = format!("/v1/mindmaps/{map}/checkpoints");
    let (s, c) = app.post(&app.worker, &route, request.clone()).await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    assert_eq!(app.post(&app.worker, &route, request.clone()).await.1, c);
    let store = app.open_store();
    let (_, read) = store
        .create_token("reader", &["read".into()], None, 10000, None, None)
        .unwrap();
    let (_, foreign) = store
        .create_token(
            "foreign",
            &["read".into(), "write".into()],
            Some(&["elsewhere".into()]),
            10000,
            None,
            None,
        )
        .unwrap();
    assert_eq!(
        app.post(&read, &route, request.clone()).await.0,
        StatusCode::FORBIDDEN
    );
    for path in [
        format!("/v1/mindmaps/{map}/versions"),
        format!("/v1/mindmaps/{map}/versions/{}", h["head"]),
        format!("/v1/mindmaps/{map}/versions/{}/state", h["head"]),
    ] {
        assert_eq!(app.get(&foreign, &path).await.0, StatusCode::FORBIDDEN);
    }
    assert_eq!(
        app.post(&foreign, &route, request.clone()).await.0,
        StatusCode::FORBIDDEN
    );
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"text":"New scope"}),
    )
    .await;
    assert_eq!(
        app.post(&app.worker, &route, request).await.0,
        StatusCode::CONFLICT
    );
    let latest = history(&app, &map).await;
    assert_eq!(
        app.post(
            &app.worker,
            &route,
            json!({"name":"Agreed scope","expected_version":latest["head"]})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, named) = app
        .get(
            &read,
            &format!("/v1/mindmaps/{map}/versions?checkpoints=true"),
        )
        .await;
    assert_eq!(named["total"], 1);
    assert_eq!(named["items"][0]["version"], h["head"]);
    assert_eq!(
        app.get(&read, &format!("/v1/mindmaps/{map}/versions/99999"))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.post(
            &app.worker,
            &route,
            json!({"name":" ","expected_version":latest["head"]})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.delete(&app.admin, &format!("/v1/mindmaps/{map}"))
            .await
            .0,
        StatusCode::OK
    );
    let db = rusqlite::Connection::open(app.db_path()).unwrap();
    for table in [
        "specification_versions",
        "specification_checkpoints",
        "specification_history_heads",
    ] {
        assert_eq!(
            db.query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE mindmap=?1"),
                [&map],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}
#[tokio::test]
async fn upgrade_baseline_does_not_invent_earlier_versions_or_mutate_on_read() {
    let app = TestApp::spawn().await;
    let (map, node) = fixture(&app).await;
    // Simulate pre-upgrade storage: only its compactable canonical log exists.
    let db = rusqlite::Connection::open(app.db_path()).unwrap();
    db.execute(
        "DELETE FROM specification_versions WHERE mindmap=?1",
        [&map],
    )
    .unwrap();
    assert_eq!(history(&app, &map).await["total"], 0);
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"text":"After upgrade"}),
    )
    .await;
    let page = history(&app, &map).await;
    assert_eq!(page["total"], 2);
    let (_, baseline) = app
        .get(&app.worker, &format!("/v1/mindmaps/{map}/versions/1"))
        .await;
    assert_eq!(baseline["kind"], "baseline");
    assert!(baseline["recorded_by"].is_null());
    assert_eq!(baseline["nodes"][0]["title"], "Agreement");
}

#[tokio::test]
async fn sync_retries_do_not_create_versions_and_out_of_order_edits_survive_materialization() {
    use yrs::Text;
    let app = TestApp::spawn().await;
    let (map, _) = fixture(&app).await;
    let store = app.open_store();
    let start = history(&app, &map).await["head"].as_i64().unwrap();
    let copy = Doc::new();
    for blob in store.load_collab_updates(&map).unwrap() {
        copy.transact_mut()
            .apply_update(Update::decode_v1(&blob).unwrap())
            .unwrap();
    }
    let state = copy
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    for _ in 0..3 {
        store
            .append_collab_update(&map, &state, "reconnect")
            .unwrap();
    }
    assert_eq!(history(&app, &map).await["head"], start);
    let remote = Doc::new();
    let text = remote.get_or_insert_text("pending-example");
    let first = {
        let mut tx = remote.transact_mut();
        text.insert(&mut tx, 0, "a");
        tx.encode_update_v1()
    };
    let second = {
        let mut tx = remote.transact_mut();
        text.insert(&mut tx, 1, "b");
        tx.encode_update_v1()
    };
    // Arrives before its origin. The later baseline must include this pending operation.
    store.append_collab_update(&map, &second, "remote").unwrap();
    let padding = Doc::new();
    let text = padding.get_or_insert_text("padding");
    for i in 0..65 {
        let mut tx = padding.transact_mut();
        text.insert(&mut tx, i, "x");
        store
            .append_collab_update(&map, &tx.encode_update_v1(), "padding")
            .unwrap();
    }
    store.append_collab_update(&map, &first, "remote").unwrap();
    let version = history(&app, &map).await["head"].as_i64().unwrap();
    let bytes = store.specification_version_state(&map, version).unwrap();
    let restored = Doc::new();
    restored
        .transact_mut()
        .apply_update(Update::decode_v1(&bytes).unwrap())
        .unwrap();
    use yrs::GetString;
    assert_eq!(
        restored
            .get_or_insert_text("pending-example")
            .get_string(&restored.transact()),
        "ab"
    );
    let deletion = {
        let mut tx = remote.transact_mut();
        tx.get_text("pending-example")
            .unwrap()
            .remove_range(&mut tx, 0, 1);
        tx.encode_update_v1()
    };
    store
        .append_collab_update(&map, &deletion, "remote")
        .unwrap();
    let deleted = history(&app, &map).await["head"].clone();
    for _ in 0..5 {
        store
            .append_collab_update(&map, &deletion, "heartbeat")
            .unwrap();
    }
    assert_eq!(history(&app, &map).await["head"], deleted);
}
