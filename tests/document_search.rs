mod common;
use common::TestApp;
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use takomo::store::search::{process_jobs, EmbeddingConfig};

async fn fixture(app: &TestApp) -> (String, String) {
    let (s, m) = app
        .post(
            &app.admin,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Search plan"}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{m}");
    let map = m["mindmap"]["id"].as_str().unwrap().to_owned();
    let(s,n)=app.post(&app.worker,&format!("/v1/mindmaps/{map}/nodes"),json!({"text":"Billing","notes":"Invoices become payable after thirty days.\nThe receipt belongs to the customer."})).await;
    assert_eq!(s, StatusCode::CREATED, "{n}");
    (map, n["nodes"][0]["id"].as_str().unwrap().into())
}
async fn mock_provider() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let router = axum::Router::new().route(
        "/embeddings",
        axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                let dimension = body["dimensions"]
                    .as_u64()
                    .or(body["output_dimension"].as_u64())
                    .unwrap() as usize;
                let data: Vec<_> = body["input"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        let mut vector = vec![0.0; dimension];
                        vector[0] = 1.0;
                        json!({"index":i,"embedding":vector})
                    })
                    .collect();
                axum::Json(json!({"data":data}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{addr}/embeddings"), calls, task)
}
fn config(endpoint: &str) -> EmbeddingConfig {
    EmbeddingConfig {
        provider: "openai".into(),
        endpoint: endpoint.into(),
        model: "fixture-v1".into(),
        dimensions: 3,
        ..Default::default()
    }
}

#[tokio::test]
async fn keyword_fallback_permissions_and_manual_sync_reuse() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, node) = fixture(&app).await;
    let path = format!("/v1/mindmaps/{map}/search?q=invoice");
    let (s, result) = app.get(&app.worker, &path).await;
    assert_eq!(s, StatusCode::OK, "{result}");
    assert_eq!(result["mode"], "keyword");
    assert_eq!(result["semantic_status"], "unconfigured");
    // FTS uses actual word tokens, not undocumented prefix matching.
    let (_, result) = app
        .get(
            &app.worker,
            &format!("/v1/mindmaps/{map}/search?q=Invoices"),
        )
        .await;
    assert_eq!(result["results"][0]["node_id"], node);
    assert_eq!(result["results"][0]["highlights"], json!(["invoices"]));
    let scoped = app.mint("other", &["read", "write"], Some(&["other"]));
    assert_eq!(app.get(&scoped, &path).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        app.get(&app.worker, "/v1/settings/embeddings").await.0,
        StatusCode::FORBIDDEN
    );
    let scoped_admin = app.mint("project-admin", &["read", "write", "admin"], Some(&["tp"]));
    assert_eq!(
        app.get(&scoped_admin, "/v1/settings/embeddings").await.0,
        StatusCode::FORBIDDEN
    );
    let readonly = app.mint("reader", &["read"], Some(&["tp"]));
    assert_eq!(
        app.post(
            &readonly,
            &format!("/v1/mindmaps/{map}/search/sync"),
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (endpoint, calls, task) = mock_provider().await;
    let mut setting = serde_json::to_value(config(&endpoint)).unwrap();
    setting["api_key"] = json!("private-test-key");
    let (s, saved) = app
        .put(&app.admin, "/v1/settings/embeddings", setting)
        .await;
    assert_eq!(s, StatusCode::OK, "{saved}");
    assert_eq!(saved["configured"], true);
    assert!(saved.get("api_key").is_none());
    let manual = format!("/v1/mindmaps/{map}/search/sync");
    assert_eq!(
        app.post(&app.worker, &manual, json!({})).await.0,
        StatusCode::OK
    );
    let store = app.open_store();
    process_jobs(&store).await.unwrap();
    let count = calls.load(Ordering::SeqCst);
    assert!(count > 0);
    app.post(&app.worker, &manual, json!({})).await;
    process_jobs(&store).await.unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        count,
        "unchanged source is not embedded again"
    );
    let (_, result) = app
        .get(
            &app.worker,
            &format!("/v1/mindmaps/{map}/search?q=settlement"),
        )
        .await;
    assert_eq!(result["mode"], "hybrid");
    assert_eq!(result["results"][0]["node_id"], node);
    assert_eq!(result["results"][0]["match_kind"], "semantic");
    assert_eq!(result["results"][0]["highlights"], json!([]));
    task.abort();
    let (_, result) = app
        .get(
            &app.worker,
            &format!("/v1/mindmaps/{map}/search?q=Invoices"),
        )
        .await;
    assert_eq!(result["mode"], "keyword");
    assert_eq!(result["semantic_status"], "unavailable");
    assert_eq!(result["results"][0]["node_id"], node);
}

#[tokio::test]
async fn durable_per_node_debounce_stale_results_and_provider_generations() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, node) = fixture(&app).await;
    let store = app.open_store();
    let setting = config("http://127.0.0.1:9/embeddings");
    store
        .save_embedding_config(setting.clone(), Some("test".into()))
        .unwrap();
    let now = takomo::ids::now_ms();
    store.refresh_search(&map, false, now).unwrap();
    assert!(store.claim_embedding_job(now + 59_999).unwrap().is_none());
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"notes":"Invoices changed"}),
    )
    .await;
    store.refresh_search(&map, false, now + 50_000).unwrap();
    assert!(store.claim_embedding_job(now + 100_000).unwrap().is_none());
    let job = store.claim_embedding_job(now + 110_000).unwrap().unwrap();
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"notes":"Invoices changed again"}),
    )
    .await;
    assert!(!store
        .finish_embedding_job(&job, Ok(&vec![vec![1.0, 0.0, 0.0]; job.chunks.len()]))
        .unwrap());
    // Repeated edits postpone the quiet deadline but never the original maximum wait.
    for time in [160_000, 210_000, 260_000] {
        app.patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}"),
            json!({"notes":format!("Invoices revision {time}")}),
        )
        .await;
        store.refresh_search(&map, false, now + time).unwrap();
    }
    let job = store
        .claim_embedding_job(now + 300_000)
        .unwrap()
        .expect("max wait caps postponement");
    let mut next = setting.clone();
    next.model = "fixture-v2".into();
    store.save_embedding_config(next.clone(), None).unwrap();
    assert!(!store
        .finish_embedding_job(&job, Ok(&vec![vec![1.0, 0.0, 0.0]; job.chunks.len()]))
        .unwrap());
    assert!(store.embedding_config().unwrap().1 == "test");
    next.endpoint = "http://127.0.0.1:10/embeddings".into();
    let saved = store.save_embedding_config(next, None).unwrap();
    assert_eq!(
        saved["configured"], false,
        "endpoint changes do not inherit another provider key"
    );
    drop(store);
    let reopened = app.open_store();
    assert!(
        reopened.search_status(&map).unwrap()["queued"]
            .as_i64()
            .unwrap()
            > 0,
        "queue survives reopen"
    );
    assert!(!reopened.load_collab_updates(&map).unwrap().is_empty());
}

#[tokio::test]
async fn additive_upgrade_preserves_source_and_is_idempotent_and_transactional() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, _) = fixture(&app).await;
    let source = app.open_store().load_collab_updates(&map).unwrap();
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    // Simulate the previously shipped database by dropping only the new derived objects.
    conn.execute_batch("DROP TRIGGER search_crdt_dirty;DROP TRIGGER search_chunks_insert;DROP TRIGGER search_chunks_delete;DROP TABLE search_fts;DROP TABLE embedding_jobs;DROP TABLE search_chunks;DROP TABLE search_nodes;DROP TABLE search_dirty_maps;DROP TABLE embedding_settings;").unwrap();
    for _ in 0..2 {
        let upgraded = app.open_store();
        assert_eq!(upgraded.load_collab_updates(&map).unwrap(), source);
        upgraded
            .refresh_search(&map, false, takomo::ids::now_ms())
            .unwrap();
        assert!(!upgraded
            .search_document(&map, "Invoices", None)
            .unwrap()
            .0
            .is_empty());
    }
    let broken = rusqlite::Connection::open_in_memory().unwrap();
    broken.execute_batch("CREATE TABLE mindmaps(id TEXT PRIMARY KEY); CREATE TABLE crdt_updates(object_kind TEXT,object_id TEXT,created_at INTEGER); CREATE TABLE search_chunks(bad TEXT);").unwrap();
    assert!(takomo::store::search::migrate(&broken).is_err());
    let count: i64 = broken
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='embedding_settings'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "failed migration rolls back all new tables");
}

#[tokio::test]
async fn unrelated_edits_do_not_starve_jobs_and_query_rechecks_source_and_config() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, node) = fixture(&app).await;
    let store = app.open_store();
    let setting = config("http://127.0.0.1:9/embeddings");
    store
        .save_embedding_config(setting.clone(), Some("test".into()))
        .unwrap();
    store
        .refresh_search(&map, true, takomo::ids::now_ms())
        .unwrap();
    let job = store
        .claim_embedding_job(takomo::ids::now_ms())
        .unwrap()
        .unwrap();
    assert_eq!(job.node_id, node);
    app.post(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes"),
        json!({"text":"Unrelated", "notes":"Separate typing"}),
    )
    .await;
    assert!(store
        .finish_embedding_job(&job, Ok(&vec![vec![1.0, 0.0, 0.0]; job.chunks.len()]))
        .unwrap());
    let vector = vec![1.0, 0.0, 0.0];
    assert!(
        store
            .search_document(&map, "settlement", Some((&vector, setting.fingerprint())))
            .unwrap()
            .1
    );
    // Simulate content changing while a query embedding is being computed.
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"notes":"New source revised"}),
    )
    .await;
    let (hits, _) = store
        .search_document(&map, "Invoices", Some((&vector, setting.fingerprint())))
        .unwrap();
    assert!(
        hits.is_empty(),
        "old source chunks cannot survive query completion"
    );
    assert_eq!(
        store.search_document(&map, "revised", None).unwrap().0[0].node_id,
        node
    );
    let mut changed = setting.clone();
    changed.model = "another-generation".into();
    store.save_embedding_config(changed, None).unwrap();
    let (hits, used) = store
        .search_document(&map, "revised", Some((&vector, setting.fingerprint())))
        .unwrap();
    assert!(!used, "in-flight query generation is discarded");
    assert_eq!(hits[0].match_kind, "keyword");
}

#[tokio::test]
async fn provider_protocol_rejects_malformed_vectors_and_preserves_order() {
    for (payload, valid) in [
        (
            json!({"data":[{"index":1,"embedding":[0,1,0]},{"index":0,"embedding":[1,0,0]}]}),
            true,
        ),
        (
            json!({"data":[{"index":0,"embedding":[1,0,0]},{"index":0,"embedding":[1,0,0]}]}),
            false,
        ),
        (
            json!({"data":[{"index":0,"embedding":[1,0]},{"index":1,"embedding":[1,0,0]}]}),
            false,
        ),
        (
            json!({"data":[{"index":0,"embedding":[0,0,0]},{"index":1,"embedding":[1,0,0]}]}),
            false,
        ),
        (json!({"data":[]}), false),
    ] {
        let router = axum::Router::new().route(
            "/embeddings",
            axum::routing::post(move || {
                let body = payload.clone();
                async move { axum::Json(body) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/embeddings", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let result = takomo::embeddings::embed(
            &config(&endpoint),
            "mock-key",
            &["first".into(), "second".into()],
            false,
        )
        .await;
        assert_eq!(result.is_ok(), valid);
        if valid {
            assert_eq!(
                result.unwrap(),
                vec![vec![1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0]]
            );
        }
        task.abort();
    }
}

#[test]
fn chunks_keep_paragraphs_and_bound_long_unicode_text() {
    let first = "ü".repeat(1500);
    let second = "β".repeat(1000);
    assert_eq!(
        takomo::store::search::chunks(&format!("{first}\n{second}")),
        vec![first, second]
    );
    let parts = takomo::store::search::chunks(&"界".repeat(4500));
    assert_eq!(
        parts.iter().map(|p| p.chars().count()).collect::<Vec<_>>(),
        vec![2000, 2000, 500]
    );
}
