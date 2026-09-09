mod common;
use common::TestApp;
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use takomo::store::search::{process_jobs, EmbeddingConfig, MAX_ATTEMPTS, RESULT_LIMIT};

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
    let store = Arc::new(app.open_store());
    process_jobs(store.clone()).await.unwrap();
    let count = calls.load(Ordering::SeqCst);
    assert!(count > 0);
    app.post(&app.worker, &manual, json!({})).await;
    process_jobs(store.clone()).await.unwrap();
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
            .hits
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
            .used_vectors
    );
    // Simulate content changing while a query embedding is being computed.
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"notes":"New source revised"}),
    )
    .await;
    let outcome = store
        .search_document(&map, "Invoices", Some((&vector, setting.fingerprint())))
        .unwrap();
    assert!(
        outcome.hits.is_empty(),
        "old source chunks cannot survive query completion"
    );
    assert_eq!(
        store.search_document(&map, "revised", None).unwrap().hits[0].node_id,
        node
    );
    let mut changed = setting.clone();
    changed.model = "another-generation".into();
    store.save_embedding_config(changed, None).unwrap();
    let outcome = store
        .search_document(&map, "revised", Some((&vector, setting.fingerprint())))
        .unwrap();
    assert!(
        !outcome.used_vectors,
        "in-flight query generation is discarded"
    );
    assert_eq!(outcome.hits[0].match_kind, "keyword");
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

#[tokio::test]
async fn missing_query_is_a_structured_error_and_unicode_excerpts_stay_aligned() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, node) = fixture(&app).await;
    let (s, body) = app
        .get(&app.worker, &format!("/v1/mindmaps/{map}/search"))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "search.query");
    assert!(body["message"].as_str().unwrap().contains("'q'"));
    assert!(body["remedy"].is_string());
    let (s, body) = app
        .get(&app.worker, &format!("/v1/mindmaps/{map}/search?q="))
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["results"], json!([]));
    assert_eq!(body["truncated"], false);
    let notes = format!("{} marker {}", "İ".repeat(100), "x".repeat(300));
    let (s, patched) = app
        .patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}"),
            json!({"notes": notes}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{patched}");
    let (_, body) = app
        .get(&app.worker, &format!("/v1/mindmaps/{map}/search?q=marker"))
        .await;
    let hit = &body["results"][0];
    assert_eq!(hit["node_id"], node);
    assert!(
        hit["excerpt"].as_str().unwrap().contains("marker"),
        "excerpt window is located by character, not by folded byte offset: {hit}"
    );
    assert_eq!(hit["highlights"], json!(["marker"]));
    assert_eq!(hit["excerpt"].as_str().unwrap().chars().count(), 280);
}

#[tokio::test]
async fn results_say_what_a_bounded_search_left_out() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, _) = fixture(&app).await;
    let nodes: Vec<Value> = (0..RESULT_LIMIT + 5)
        .map(|i| json!({"text": format!("Section {i}"), "notes": format!("Parcel {i} ships on day {i}.")}))
        .collect();
    let (s, added) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({"nodes": nodes}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{added}");
    let (s, body) = app
        .get(&app.worker, &format!("/v1/mindmaps/{map}/search?q=parcel"))
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["results"].as_array().unwrap().len(), RESULT_LIMIT);
    assert_eq!(body["limit"], RESULT_LIMIT);
    assert_eq!(body["candidates"], RESULT_LIMIT + 5);
    assert_eq!(body["truncated"], true);
    let note = body["note"].as_str().expect("a truncated page says so");
    assert!(
        note.contains(&format!(
            "{RESULT_LIMIT} best-ranked of {}",
            RESULT_LIMIT + 5
        )),
        "{note}"
    );
    let (_, body) = app
        .get(&app.worker, &format!("/v1/mindmaps/{map}/search?q=receipt"))
        .await;
    assert_eq!(body["results"].as_array().unwrap().len(), 1);
    assert_eq!(body["candidates"], 1);
    assert_eq!(body["truncated"], false);
    assert!(body.get("note").is_none());
}

#[tokio::test]
async fn retries_are_capped_and_reset_by_content_config_or_manual_sync() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, node) = fixture(&app).await;
    let store = app.open_store();
    let setting = config("http://127.0.0.1:9/embeddings");
    store
        .save_embedding_config(setting.clone(), Some("test".into()))
        .unwrap();
    let mut now = takomo::ids::now_ms();
    store.refresh_search(&map, true, now).unwrap();
    let exhaust = |store: &takomo::store::Store, now: &mut i64| {
        for attempt in 0..MAX_ATTEMPTS {
            let job = store
                .claim_embedding_job(*now)
                .unwrap()
                .unwrap_or_else(|| panic!("attempt {attempt} is still allowed"));
            assert!(store
                .finish_embedding_job(&job, Err("provider refused the batch"))
                .unwrap());
            *now += 3_600_000;
        }
        assert!(
            store.claim_embedding_job(*now).unwrap().is_none(),
            "a job that failed {MAX_ATTEMPTS} times is parked, not resent every backoff"
        );
    };
    exhaust(&store, &mut now);
    let status = store.search_status(&map).unwrap();
    assert_eq!(status["failed"], 1);
    assert_eq!(status["queued"], 1);
    assert_eq!(status["pending"], 0);
    assert_eq!(status["running"], 0);
    assert!(status["last_synced_at"].is_null());
    assert_eq!(status["last_error"], "provider refused the batch");
    // Manual sync resets the cap and the error it was parked with, like the other two resets.
    store.refresh_search(&map, true, now).unwrap();
    let reset = store.search_status(&map).unwrap();
    assert_eq!(reset["failed"], 0);
    assert_eq!(reset["queued"], 1);
    assert_eq!(reset["pending"], 1);
    assert_eq!(reset["last_error"], Value::Null);
    exhaust(&store, &mut now);
    // A content change resets it.
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"notes":"Invoices after the outage"}),
    )
    .await;
    now += 1;
    store.refresh_search(&map, false, now).unwrap();
    now += setting.quiet_seconds * 1000 + 1;
    exhaust(&store, &mut now);
    // A provider generation change resets it.
    let mut next = setting.clone();
    next.model = "fixture-v2".into();
    store.save_embedding_config(next, None).unwrap();
    assert!(store.claim_embedding_job(now).unwrap().is_some());
    // The HTTP surface reports the same cap after the worker gives up.
    let manual = format!("/v1/mindmaps/{map}/search/sync");
    assert_eq!(
        app.post(&app.worker, &manual, json!({})).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn query_embeddings_are_bounded_per_token_with_keyword_fallback() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, node) = fixture(&app).await;
    let (endpoint, calls, task) = mock_provider().await;
    let mut setting = serde_json::to_value(config(&endpoint)).unwrap();
    setting["api_key"] = json!("private-test-key");
    assert_eq!(
        app.put(&app.admin, "/v1/settings/embeddings", setting)
            .await
            .0,
        StatusCode::OK
    );
    app.post(
        &app.worker,
        &format!("/v1/mindmaps/{map}/search/sync"),
        json!({}),
    )
    .await;
    process_jobs(Arc::new(app.open_store())).await.unwrap();
    let indexed = calls.load(Ordering::SeqCst);
    let path = format!("/v1/mindmaps/{map}/search?q=invoices");
    let limit = takomo::api::search::QUERY_EMBEDDINGS_PER_MINUTE as usize;
    for i in 0..limit {
        let distinct = format!("{path}%20{i}");
        let (s, body) = app.get(&app.worker, &distinct).await;
        assert_eq!(s, StatusCode::OK, "{body}");
        assert_eq!(body["mode"], "hybrid");
        assert_eq!(body["semantic_status"], "ready");
    }
    assert_eq!(calls.load(Ordering::SeqCst), indexed + limit);
    let (s, body) = app.get(&app.worker, &path).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["mode"], "keyword");
    assert_eq!(body["semantic_status"], "throttled");
    assert_eq!(body["results"][0]["node_id"], node);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        indexed + limit,
        "an exhausted budget answers from keywords without a provider call"
    );
    let (_, cached) = app.get(&app.worker, &format!("{path}%200")).await;
    assert_eq!(
        cached["mode"], "hybrid",
        "cache hits bypass exhausted outbound budget"
    );
    assert_eq!(calls.load(Ordering::SeqCst), indexed + limit);
    // The budget is per token: another credential still gets its semantic pass.
    let (_, body) = app.get(&app.worker2, &path).await;
    assert_eq!(body["mode"], "hybrid");
    assert_eq!(calls.load(Ordering::SeqCst), indexed + limit + 1);
    task.abort();
}

#[tokio::test]
async fn a_corrupt_map_does_not_stall_indexing_of_healthy_maps() {
    let app = TestApp::spawn_without_sweeper().await;
    let (broken, broken_node) = fixture(&app).await;
    let (s, project) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({"id":"healthy","name":"Healthy"}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{project}");
    let (s, m) = app
        .post(
            &app.admin,
            "/v1/mindmaps",
            json!({"project":"healthy","title":"Healthy plan"}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{m}");
    let healthy = m["mindmap"]["id"].as_str().unwrap().to_owned();
    let (s, n) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{healthy}/nodes"),
            json!({"text":"Shipping","notes":"Parcels leave the warehouse daily."}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{n}");
    let (endpoint, calls, task) = mock_provider().await;
    let mut setting = serde_json::to_value(config(&endpoint)).unwrap();
    setting["api_key"] = json!("private-test-key");
    assert_eq!(
        app.put(&app.admin, "/v1/settings/embeddings", setting)
            .await
            .0,
        StatusCode::OK
    );
    let store = Arc::new(app.open_store());
    store
        .refresh_search(&broken, true, takomo::ids::now_ms())
        .unwrap();
    store
        .refresh_search(&healthy, true, takomo::ids::now_ms())
        .unwrap();
    // The broken map's most recent update is garbage, and it sorts first as the oldest dirty map.
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    conn.execute(
        "INSERT INTO crdt_updates(object_kind,object_id,blob,bytes,created_by,created_at)VALUES('mindmap',?1,x'FFFFFFFFFFFFFFFF',8,'test',1)",
        [&broken],
    )
    .unwrap();
    conn.execute(
        "UPDATE search_dirty_maps SET changed_at=1 WHERE map_id=?1",
        [&broken],
    )
    .unwrap();
    app.patch(
        &app.worker,
        &format!(
            "/v1/mindmaps/{healthy}/nodes/{}",
            n["nodes"][0]["id"].as_str().unwrap()
        ),
        json!({"notes":"Parcels leave the warehouse twice daily."}),
    )
    .await;
    store
        .refresh_search(&healthy, true, takomo::ids::now_ms())
        .unwrap();
    process_jobs(store.clone())
        .await
        .expect("one poisoned map does not fail the pass");
    assert!(
        calls.load(Ordering::SeqCst) > 0,
        "the healthy map's job ran"
    );
    let status = store.search_status(&healthy).unwrap();
    assert_eq!(status["indexed"], status["total"], "{status}");
    assert_eq!(status["last_error"], Value::Null);
    let status = store.search_status(&broken).unwrap();
    let recorded = status["last_error"]
        .as_str()
        .expect("the poisoned map's failure is recorded");
    assert!(recorded.contains("source document"), "{status}");
    // Its failure is surfaced over HTTP, and keyword search still answers from the last good projection.
    let (s, over_http) = app
        .get(&app.worker, &format!("/v1/mindmaps/{broken}/search/status"))
        .await;
    assert_eq!(s, StatusCode::OK, "{over_http}");
    assert_eq!(over_http["last_error"], recorded);
    let (s, body) = app
        .get(
            &app.worker,
            &format!("/v1/mindmaps/{broken}/search?q=invoices"),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["results"][0]["node_id"], broken_node);
    // A second pass does not keep re-failing the same map: it is no longer dirty.
    process_jobs(store.clone()).await.unwrap();
    task.abort();
}

async fn second_map(app: &TestApp, project: &str) -> String {
    let (s, created) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({"id": project, "name": project}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    let (s, m) = app
        .post(
            &app.admin,
            "/v1/mindmaps",
            json!({"project": project, "title": "Second plan"}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{m}");
    m["mindmap"]["id"].as_str().unwrap().to_owned()
}
fn poison(app: &TestApp, map: &str) {
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    conn.execute(
        "INSERT INTO crdt_updates(object_kind,object_id,blob,bytes,created_by,created_at)VALUES('mindmap',?1,x'FFFFFFFFFFFFFFFF',8,'test',1)",
        [map],
    )
    .unwrap();
}

#[tokio::test]
async fn first_read_after_poison_answers_from_the_last_good_projection_and_says_so() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, node) = fixture(&app).await;
    let (s, warm) = app
        .get(
            &app.worker,
            &format!("/v1/mindmaps/{map}/search?q=invoices"),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{warm}");
    assert_eq!(warm["projection"], "current");
    assert_eq!(warm["projection_error"], Value::Null);
    poison(&app, &map);
    // The very first read after the failure: not a 500, the last good projection, flagged.
    let (s, first) = app
        .get(
            &app.worker,
            &format!("/v1/mindmaps/{map}/search?q=invoices"),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{first}");
    assert_eq!(first["results"][0]["node_id"], node);
    assert_eq!(first["projection"], "stale");
    let reason = first["projection_error"]
        .as_str()
        .expect("the failure is named");
    assert!(reason.contains("source document"), "{first}");
    let (s, status) = app
        .get(&app.worker, &format!("/v1/mindmaps/{map}/search/status"))
        .await;
    assert_eq!(s, StatusCode::OK, "{status}");
    assert_eq!(status["projection"], "stale");
    assert_eq!(status["last_error"], reason);
    // A map that never projected answers empty rather than pretending, on its first read too.
    let fresh = second_map(&app, "fresh").await;
    poison(&app, &fresh);
    let (s, status) = app
        .get(&app.worker, &format!("/v1/mindmaps/{fresh}/search/status"))
        .await;
    assert_eq!(s, StatusCode::OK, "{status}");
    assert_eq!(status["projection"], "stale");
    assert_eq!(status["total"], 0);
    let (s, empty) = app
        .get(&app.worker, &format!("/v1/mindmaps/{fresh}/search?q=plan"))
        .await;
    assert_eq!(s, StatusCode::OK, "{empty}");
    assert_eq!(empty["results"], json!([]));
    assert_eq!(empty["projection"], "stale");
    assert!(empty["projection_error"].is_string());
}

#[tokio::test]
async fn clean_reads_take_no_write_and_signal_no_change() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, node) = fixture(&app).await;
    let store = app.open_store();
    let mut changes = store.changes.subscribe();
    let now = takomo::ids::now_ms();
    store.refresh_search(&map, false, now).unwrap();
    assert!(
        changes.has_changed().unwrap(),
        "a dirty map is projected in a write"
    );
    changes.borrow_and_update();
    store.refresh_search(&map, false, now).unwrap();
    assert!(store.project_search(&map, now).unwrap().is_none());
    let outcome = store.search_document(&map, "invoices", None).unwrap();
    assert_eq!(outcome.hits[0].node_id, node);
    assert_eq!(store.search_status(&map).unwrap()["projection"], "current");
    assert!(
        !changes.has_changed().unwrap(),
        "a clean read neither commits nor tells open sockets to refresh"
    );
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"notes":"Invoices are now due at once."}),
    )
    .await;
    changes.borrow_and_update();
    assert_eq!(
        store.search_document(&map, "once", None).unwrap().hits[0].node_id,
        node,
        "a changed map is still projected before the read"
    );
    assert!(changes.has_changed().unwrap());
}

#[tokio::test]
async fn manual_sync_on_an_archived_project_uses_the_project_archive_contract() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, node) = fixture(&app).await;
    let (s, archived) = app
        .post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{archived}");
    let (s, refused) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/search/sync"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "project.archived");
    assert_eq!(refused["details"]["project"], "tp");
    assert!(refused["details"]["archived_at"].is_string());
    assert!(refused["message"].as_str().unwrap().contains("unarchive"));
    let (s, read) = app
        .get(
            &app.worker,
            &format!("/v1/mindmaps/{map}/search?q=invoices"),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{read}");
    assert_eq!(read["results"][0]["node_id"], node);
    assert_eq!(
        app.get(&app.worker, &format!("/v1/mindmaps/{map}/search/status"))
            .await
            .0,
        StatusCode::OK
    );
    let (s, restored) = app
        .post(&app.admin, "/v1/projects/tp/unarchive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{restored}");
    assert_eq!(
        app.post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/search/sync"),
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn symbol_only_queries_spend_nothing_and_stay_truthful() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, _) = fixture(&app).await;
    let (endpoint, calls, task) = mock_provider().await;
    let mut setting = serde_json::to_value(config(&endpoint)).unwrap();
    setting["api_key"] = json!("private-test-key");
    assert_eq!(
        app.put(&app.admin, "/v1/settings/embeddings", setting)
            .await
            .0,
        StatusCode::OK
    );
    app.post(
        &app.worker,
        &format!("/v1/mindmaps/{map}/search/sync"),
        json!({}),
    )
    .await;
    process_jobs(Arc::new(app.open_store())).await.unwrap();
    let indexed = calls.load(Ordering::SeqCst);
    for symbols in ["%3F%21", "%F0%9F%9A%80", "%20%2D%2D%20"] {
        let (s, body) = app
            .get(
                &app.worker,
                &format!("/v1/mindmaps/{map}/search?q={symbols}"),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{body}");
        assert_eq!(body["results"], json!([]));
        assert_eq!(body["mode"], "keyword");
        assert_eq!(body["semantic_status"], "ready", "{body}");
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        indexed,
        "nothing to embed means no provider call"
    );
    // ...and no budget spent: the full per-token allowance is still available.
    let path = format!("/v1/mindmaps/{map}/search?q=invoices");
    for _ in 0..takomo::api::search::QUERY_EMBEDDINGS_PER_MINUTE {
        let (_, body) = app.get(&app.worker, &path).await;
        assert_eq!(body["semantic_status"], "ready", "{body}");
        assert_eq!(body["mode"], "hybrid");
    }
    task.abort();
}

#[tokio::test]
async fn key_only_put_is_refused_before_anything_changes() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, _) = fixture(&app).await;
    let (endpoint, calls, task) = mock_provider().await;
    let mut setting = serde_json::to_value(config(&endpoint)).unwrap();
    setting["api_key"] = json!("private-test-key");
    assert_eq!(
        app.put(&app.admin, "/v1/settings/embeddings", setting)
            .await
            .0,
        StatusCode::OK
    );
    app.post(
        &app.worker,
        &format!("/v1/mindmaps/{map}/search/sync"),
        json!({}),
    )
    .await;
    let store = Arc::new(app.open_store());
    process_jobs(store.clone()).await.unwrap();
    let indexed_calls = calls.load(Ordering::SeqCst);
    let (before_config, before_key) = store.embedding_config().unwrap();
    let before_status = store.search_status(&map).unwrap();
    assert_eq!(before_status["indexed"], before_status["total"]);
    let (s, refused) = app
        .put(
            &app.admin,
            "/v1/settings/embeddings",
            json!({"api_key":"rotated"}),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert_eq!(refused["code"], "embeddings.config");
    let message = refused["message"].as_str().unwrap();
    assert!(
        message.contains("provider") && message.contains("Nothing was changed"),
        "{message}"
    );
    assert!(refused["remedy"].is_string());
    let (after_config, after_key) = store.embedding_config().unwrap();
    assert_eq!(
        after_key, before_key,
        "the old key is neither replaced nor cleared"
    );
    assert_eq!(after_config.endpoint, before_config.endpoint);
    assert_eq!(after_config.fingerprint(), before_config.fingerprint());
    assert_eq!(
        store.search_status(&map).unwrap(),
        before_status,
        "no job was re-queued"
    );
    let (_, shown) = app.get(&app.admin, "/v1/settings/embeddings").await;
    assert_eq!(shown["provider"], "openai");
    assert_eq!(shown["endpoint"], endpoint);
    process_jobs(store.clone()).await.unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        indexed_calls,
        "nothing was sent anywhere"
    );
    // The documented remedy works as written: GET, edit one field, PUT the whole object back.
    let (_, mut edited) = app.get(&app.admin, "/v1/settings/embeddings").await;
    assert_eq!(edited["configured"], true);
    edited["model"] = json!("fixture-v1b");
    let (s, saved) = app.put(&app.admin, "/v1/settings/embeddings", edited).await;
    assert_eq!(s, StatusCode::OK, "{saved}");
    assert_eq!(saved["model"], "fixture-v1b");
    assert_eq!(saved["configured"], true);
    assert!(saved.get("api_key").is_none());
    assert_eq!(
        store.embedding_config().unwrap().1,
        before_key,
        "the key survived a round trip that did not mention it"
    );
    // A complete replacement still works, and rotates the key with it.
    let mut full = serde_json::to_value(config(&endpoint)).unwrap();
    full["model"] = json!("fixture-v2");
    full["api_key"] = json!("rotated");
    let (s, saved) = app.put(&app.admin, "/v1/settings/embeddings", full).await;
    assert_eq!(s, StatusCode::OK, "{saved}");
    assert_eq!(saved["model"], "fixture-v2");
    assert_eq!(saved["configured"], true);
    assert_eq!(store.embedding_config().unwrap().1, "rotated");
    task.abort();
}

#[tokio::test]
async fn manual_sync_says_whether_its_bypass_was_applied() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, node) = fixture(&app).await;
    let store = app.open_store();
    let setting = config("http://127.0.0.1:9/embeddings");
    store
        .save_embedding_config(setting.clone(), Some("test".into()))
        .unwrap();
    let path = format!("/v1/mindmaps/{map}/search/sync");
    let (s, body) = app.post(&app.worker, &path, json!({})).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["sync"], "scheduled");
    assert!(body.get("sync_note").is_none());
    assert_eq!(body["projection"], "current");
    // Park the node, then let an edit outrun a manual projection.
    let mut now = takomo::ids::now_ms();
    for _ in 0..MAX_ATTEMPTS {
        let job = store.claim_embedding_job(now).unwrap().unwrap();
        assert!(store.finish_embedding_job(&job, Err("outage")).unwrap());
        now += 3_600_000;
    }
    assert_eq!(store.search_status(&map).unwrap()["failed"], 1);
    let projection = store.compute_projection(&map).unwrap();
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"notes":"Invoices, typed while syncing"}),
    )
    .await;
    assert!(
        !store
            .apply_projection(&map, &projection, now, true)
            .unwrap(),
        "a manual projection outrun by an edit is declined like any other"
    );
    let status = store.search_status(&map).unwrap();
    assert_eq!(
        status["failed"], 1,
        "a declined manual projection applies no bypass: parked stays parked"
    );
    assert_eq!(status["projection"], "stale");
    let deferred = takomo::api::search::sync_response(status, false);
    assert_eq!(deferred["sync"], "deferred");
    assert!(deferred["sync_note"]
        .as_str()
        .unwrap()
        .starts_with("Nothing was scheduled"));
    // Once editing pauses the same call schedules the bypass and un-parks the node.
    let (s, body) = app.post(&app.worker, &path, json!({})).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["sync"], "scheduled");
    assert_eq!(body["failed"], 0);
    assert_eq!(body["projection"], "current");
    assert!(store
        .claim_embedding_job(takomo::ids::now_ms())
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn an_ordinary_reopen_does_not_redirty_projected_maps() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, node) = fixture(&app).await;
    let empty = second_map(&app, "empty").await;
    let now = takomo::ids::now_ms();
    {
        let store = app.open_store();
        store.refresh_search(&map, false, now).unwrap();
        store.refresh_search(&empty, false, now).unwrap();
        assert_eq!(store.search_status(&empty).unwrap()["total"], 0);
    }
    let reopened = app.open_store();
    let changes = reopened.changes.subscribe();
    reopened.refresh_search(&map, false, now).unwrap();
    reopened.refresh_search(&empty, false, now).unwrap();
    assert!(
        !changes.has_changed().unwrap(),
        "a reopen schedules no replay for a map that is already projected, even an empty one"
    );
    assert_eq!(
        reopened
            .search_document(&map, "invoices", None)
            .unwrap()
            .hits[0]
            .node_id,
        node
    );
    assert_eq!(
        reopened.search_status(&empty).unwrap()["projection"],
        "current"
    );
    // The trigger still marks a map whose log grows after the reopen.
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"notes":"Invoices after restart"}),
    )
    .await;
    reopened.refresh_search(&map, false, now + 1).unwrap();
    assert!(changes.has_changed().unwrap());
    assert_eq!(
        reopened
            .search_document(&map, "restart", None)
            .unwrap()
            .hits[0]
            .node_id,
        node
    );
}

#[tokio::test]
async fn idle_worker_passes_write_nothing_and_a_due_job_is_still_claimed() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, _) = fixture(&app).await;
    let store = Arc::new(app.open_store());
    let now = takomo::ids::now_ms();
    store.refresh_search(&map, false, now).unwrap();
    let mut changes = store.changes.subscribe();
    process_jobs(store.clone()).await.unwrap();
    assert!(store.claim_embedding_job(now).unwrap().is_none());
    assert!(
        !changes.has_changed().unwrap(),
        "an unconfigured worker pass commits nothing and signals nothing"
    );
    let setting = config("http://127.0.0.1:9/embeddings");
    store
        .save_embedding_config(setting.clone(), Some("test".into()))
        .unwrap();
    changes.borrow_and_update();
    assert!(store.claim_embedding_job(now).unwrap().is_none());
    assert!(
        !changes.has_changed().unwrap(),
        "configured with nothing due yet is still an idle pass"
    );
    let job = store
        .claim_embedding_job(now + setting.quiet_seconds * 1000 + 1)
        .unwrap()
        .expect("a due job is claimed");
    assert!(changes.has_changed().unwrap(), "a real claim is a write");
    assert!(job.lease > now);
}

#[tokio::test]
async fn a_parked_or_expired_job_gets_a_fresh_quiet_window_when_edited_again() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, node) = fixture(&app).await;
    let store = app.open_store();
    let setting = config("http://127.0.0.1:9/embeddings");
    let quiet = setting.quiet_seconds * 1000;
    let max_wait = setting.max_wait_seconds * 1000;
    store
        .save_embedding_config(setting.clone(), Some("test".into()))
        .unwrap();
    let t0 = takomo::ids::now_ms();
    store.refresh_search(&map, true, t0).unwrap();
    // Park the node: three failed attempts during an outage.
    for _ in 0..MAX_ATTEMPTS {
        let job = store.claim_embedding_job(t0 + max_wait).unwrap().unwrap();
        assert!(store.finish_embedding_job(&job, Err("outage")).unwrap());
    }
    assert!(store.claim_embedding_job(t0 + 7_200_000).unwrap().is_none());
    // An hour later the provider is back and somebody edits the node.
    let edit = t0 + 3_600_000;
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"notes":"Invoices, edited after the outage"}),
    )
    .await;
    store.refresh_search(&map, false, edit).unwrap();
    assert!(
        store
            .claim_embedding_job(edit + quiet - 1)
            .unwrap()
            .is_none(),
        "a parked row edited again waits the quiet period instead of embedding mid-typing"
    );
    let job = store
        .claim_embedding_job(edit + quiet)
        .unwrap()
        .expect("due once the fresh window's quiet period passes");
    assert_eq!(job.node_id, node);
    // A row that sat in a backlog past its maximum wait is the same case.
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"notes":"Invoices, queued behind a long backlog"}),
    )
    .await;
    let queued = edit + quiet + 1;
    store.refresh_search(&map, false, queued).unwrap();
    let late = queued + max_wait + 3_600_000;
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"notes":"Invoices, edited while the backlog was still draining"}),
    )
    .await;
    store.refresh_search(&map, false, late).unwrap();
    assert!(
        store
            .claim_embedding_job(late + quiet - 1)
            .unwrap()
            .is_none(),
        "an expired window restarts rather than making every edit due at once"
    );
    assert!(store.claim_embedding_job(late + quiet).unwrap().is_some());
    // Within one active window the maximum wait still caps postponement.
    let window = late;
    let mut t = late;
    while t + quiet < window + max_wait {
        t += quiet - 1;
        app.patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}"),
            json!({"notes":format!("Invoices, still typing at {t}")}),
        )
        .await;
        store.refresh_search(&map, false, t).unwrap();
    }
    assert!(store
        .claim_embedding_job(window + max_wait - 1)
        .unwrap()
        .is_none());
    assert!(
        store
            .claim_embedding_job(window + max_wait)
            .unwrap()
            .is_some(),
        "continuous edits within one window are still bounded by the maximum wait"
    );
    // Manual sync bypasses both.
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"notes":"Invoices, synced by hand"}),
    )
    .await;
    let now = t + 1;
    store.refresh_search(&map, true, now).unwrap();
    assert!(store.claim_embedding_job(now).unwrap().is_some());
}

#[tokio::test]
async fn a_projection_computed_before_the_log_grew_is_not_applied() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, node) = fixture(&app).await;
    let store = app.open_store();
    let now = takomo::ids::now_ms();
    let projection = store.compute_projection(&map).unwrap();
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"notes":"Invoices grew after the projection was computed"}),
    )
    .await;
    assert!(
        !store
            .apply_projection(&map, &projection, now, false)
            .unwrap(),
        "the apply step rechecks the log sequence and declines a stale projection"
    );
    assert_eq!(
        store.search_document(&map, "grew", None).unwrap().hits[0].node_id,
        node,
        "a declined projection leaves the map dirty, so the next search projects the grown log itself"
    );
    let fresh = store.compute_projection(&map).unwrap();
    assert!(fresh.seq() > projection.seq());
    assert!(store.apply_projection(&map, &fresh, now, false).unwrap());
    assert_eq!(
        store.search_document(&map, "grew", None).unwrap().hits[0].node_id,
        node
    );
    assert_eq!(store.search_status(&map).unwrap()["projection"], "current");
    // A completion whose map is still dirty when it lands is not written; the job is released, not lost.
    let setting = config("http://127.0.0.1:9/embeddings");
    store
        .save_embedding_config(setting.clone(), Some("test".into()))
        .unwrap();
    store.refresh_search(&map, true, now).unwrap();
    let job = store.claim_embedding_job(now).unwrap().unwrap();
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"notes":"Invoices changed under the embedding"}),
    )
    .await;
    assert!(!store
        .finish_embedding_job(&job, Ok(&vec![vec![1.0, 0.0, 0.0]; job.chunks.len()]))
        .unwrap());
    assert_eq!(store.search_status(&map).unwrap()["running"], 0);
}

#[tokio::test]
async fn a_completion_declined_at_finish_time_is_deferred_not_resent_and_the_pass_goes_on() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, node) = fixture(&app).await;
    let healthy = second_map(&app, "healthy").await;
    let (s, n) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{healthy}/nodes"),
            json!({"text":"Shipping","notes":"Parcels leave the warehouse daily."}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{n}");
    let (endpoint, calls, task) = mock_provider().await;
    let setting = config(&endpoint);
    let quiet = setting.quiet_seconds * 1000;
    let store = Arc::new(app.open_store());
    store
        .save_embedding_config(setting.clone(), Some("private-test-key".into()))
        .unwrap();
    let now = takomo::ids::now_ms();
    store.refresh_search(&map, true, now).unwrap();
    store.refresh_search(&healthy, true, now).unwrap();
    let job = store.claim_embedding_job(now).unwrap().unwrap();
    assert_eq!(job.map_id, map);
    // The map's log becomes unreplayable while the provider is answering.
    poison(&app, &map);
    let vectors = vec![vec![1.0, 0.0, 0.0]; job.chunks.len()];
    assert!(
        !store.finish_embedding_job(&job, Ok(&vectors)).unwrap(),
        "a completion the projection cannot vouch for is declined, not an error"
    );
    let status = store.search_status(&map).unwrap();
    assert_eq!(status["projection"], "stale");
    assert!(status["last_error"]
        .as_str()
        .unwrap()
        .contains("source document"));
    assert_eq!(status["running"], 0, "the lease is released");
    let next = store
        .claim_embedding_job(now)
        .unwrap()
        .expect("the pass moves on");
    assert_eq!(
        next.map_id, healthy,
        "the declined job is not the next claim of the same pass"
    );
    assert!(store
        .finish_embedding_job(&next, Ok(&vec![vec![1.0, 0.0, 0.0]; next.chunks.len()]))
        .unwrap());
    let after = takomo::ids::now_ms();
    assert!(
        store
            .claim_embedding_job(now + quiet - 1)
            .unwrap()
            .is_none(),
        "the declined job waits a quiet period before it is sent again"
    );
    assert_eq!(
        store
            .claim_embedding_job(after + quiet)
            .unwrap()
            .unwrap()
            .node_id,
        node
    );
    // A whole worker pass with the poisoned map still indexes the healthy one and returns Ok.
    let before = calls.load(Ordering::SeqCst);
    process_jobs(store.clone()).await.unwrap();
    let healthy_status = store.search_status(&healthy).unwrap();
    assert_eq!(healthy_status["indexed"], healthy_status["total"]);
    assert!(calls.load(Ordering::SeqCst) >= before);
    task.abort();
}

#[tokio::test]
async fn embedding_status_counts_passages_and_records_only_full_success() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, node) = fixture(&app).await;
    let long = format!(
        "{}\n{}\n{}",
        "alpha ".repeat(220),
        "beta ".repeat(250),
        "gamma ".repeat(230)
    );
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"notes":long}),
    )
    .await;
    let store = app.open_store();
    store
        .save_embedding_config(config("http://127.0.0.1:9/embeddings"), Some("test".into()))
        .unwrap();
    store
        .refresh_search(&map, true, takomo::ids::now_ms())
        .unwrap();
    let path = format!("/v1/mindmaps/{map}/search/status");
    let (code, before) = app.get(&app.worker, &path).await;
    assert_eq!(code, StatusCode::OK);
    assert!(before["passages_total"].as_i64().unwrap() > before["total"].as_i64().unwrap());
    assert_eq!(before["passages_indexed"], 0);
    assert!(before["last_synced_at"].is_null());
    assert_eq!(before["pending"], before["queued"]);
    let mut successes = 0;
    while let Some(job) = store.claim_embedding_job(takomo::ids::now_ms()).unwrap() {
        let running = store.search_status(&map).unwrap();
        assert_eq!(running["running"], 1);
        assert_eq!(
            running["pending"].as_i64().unwrap() + 1,
            running["queued"].as_i64().unwrap()
        );
        let vectors = vec![vec![1.0, 0.0, 0.0]; job.chunks.len()];
        assert!(store.finish_embedding_job(&job, Ok(&vectors)).unwrap());
        successes += 1;
        let state = store.search_status(&map).unwrap();
        if state["queued"].as_i64().unwrap() > 0 {
            assert!(state["last_synced_at"].is_null());
        }
    }
    assert!(successes > 0);
    let done = store.search_status(&map).unwrap();
    let stamp = done["last_synced_at"].as_i64().unwrap();
    assert_eq!(done["passages_indexed"], done["passages_total"]);
    assert_eq!(done["pending"], 0);
    assert_eq!(done["running"], 0);
    assert_eq!(done["failed"], 0);
    store.refresh_search(&map, true, stamp + 10000).unwrap();
    assert_eq!(store.search_status(&map).unwrap()["last_synced_at"], stamp);
    assert_eq!(app.get(&app.worker, &path).await.1["last_synced_at"], stamp);
    assert_eq!(
        app.open_store().search_status(&map).unwrap()["last_synced_at"],
        stamp
    );

    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"notes":"Updated passage"}),
    )
    .await;
    let changed = app.get(&app.worker, &path).await.1;
    assert_eq!(changed["last_synced_at"], stamp);
    assert!(changed["pending"].as_i64().unwrap() > 0);
    store
        .refresh_search(&map, true, takomo::ids::now_ms())
        .unwrap();
    let job = store
        .claim_embedding_job(takomo::ids::now_ms())
        .unwrap()
        .unwrap();
    store
        .finish_embedding_job(&job, Err("mock failure"))
        .unwrap();
    assert_eq!(store.search_status(&map).unwrap()["last_synced_at"], stamp);
    let mut replacement = config("http://127.0.0.1:9/embeddings");
    replacement.model = "another-model".into();
    store.save_embedding_config(replacement, None).unwrap();
    assert!(store.search_status(&map).unwrap()["last_synced_at"].is_null());
    assert_eq!(store.search_status(&map).unwrap()["passages_indexed"], 0);
}

#[tokio::test]
async fn embedding_history_upgrade_preserves_vectors_and_does_not_invent_a_timestamp() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, _) = fixture(&app).await;
    let store = app.open_store();
    store
        .save_embedding_config(config("http://127.0.0.1:9/embeddings"), Some("test".into()))
        .unwrap();
    store
        .refresh_search(&map, true, takomo::ids::now_ms())
        .unwrap();
    while let Some(job) = store.claim_embedding_job(takomo::ids::now_ms()).unwrap() {
        store
            .finish_embedding_job(&job, Ok(&vec![vec![1.0, 0.0, 0.0]; job.chunks.len()]))
            .unwrap();
    }
    let source = store.load_collab_updates(&map).unwrap();
    let before = store.search_status(&map).unwrap();
    assert!(before["last_synced_at"].is_number());
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    conn.execute_batch("DROP TABLE embedding_sync_history")
        .unwrap();
    for _ in 0..2 {
        let upgraded = app.open_store();
        assert_eq!(upgraded.load_collab_updates(&map).unwrap(), source);
        let status = upgraded.search_status(&map).unwrap();
        assert_eq!(status["passages_indexed"], before["passages_indexed"]);
        assert_eq!(status["queued"], 0);
        assert!(status["last_synced_at"].is_null());
        upgraded
            .refresh_search(&map, true, takomo::ids::now_ms())
            .unwrap();
        assert!(upgraded.search_status(&map).unwrap()["last_synced_at"].is_null());
    }
    let empty_id = second_map(&app, "empty-history").await;
    store
        .refresh_search(&empty_id, true, takomo::ids::now_ms())
        .unwrap();
    assert!(store.search_status(&empty_id).unwrap()["last_synced_at"].is_null());
}

#[tokio::test]
async fn query_cache_identity_expiration_capacity_and_failure_retry() {
    use std::time::Duration;
    use takomo::query_embeddings::{QueryCache, QueryEmbedding};
    let (endpoint, calls, task) = mock_provider().await;
    let cache = QueryCache::new(2, Duration::from_secs(1));
    let cfg = config(&endpoint);
    let (generation, first) = cache.get(&cfg, "secret", "alice", " Query ").await;
    assert!(matches!(first, QueryEmbedding::Ready(_)));
    cache.get(&cfg, "secret", "alice", "Query").await;
    assert_eq!(calls.load(Ordering::SeqCst), 1, "trimmed cache hit");
    cache.get(&cfg, "secret", "alice", "query").await;
    assert_eq!(calls.load(Ordering::SeqCst), 2, "case is meaningful");
    cache.get(&cfg, "secret", "bob", "Query").await;
    cache.get(&cfg, "secret", "alice", "Query").await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        4,
        "token isolation and LRU capacity"
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    cache.get(&cfg, "secret", "alice", "Query").await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        5,
        "expired vector is recomputed"
    );
    cache.get(&cfg, "replacement", "alice", "Query").await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        6,
        "credential changes cannot reuse old entries"
    );
    let mut changed = cfg.clone();
    changed.model = "fixture-v2".into();
    cache.get(&changed, "replacement", "alice", "Query").await;
    assert_eq!(calls.load(Ordering::SeqCst), 7);
    cache.invalidate();
    assert!(!cache.is_current(generation));
    cache.get(&changed, "replacement", "alice", "Query").await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        8,
        "config save invalidates even compatible vectors"
    );
    task.abort();

    let attempts = Arc::new(AtomicUsize::new(0));
    let count = attempts.clone();
    let router = axum::Router::new().route(
        "/embeddings",
        axum::routing::post(move || {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                StatusCode::SERVICE_UNAVAILABLE
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bad = config(&format!(
        "http://{}/embeddings",
        listener.local_addr().unwrap()
    ));
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    for _ in 0..2 {
        assert!(matches!(
            cache.get(&bad, "secret", "alice", "Query").await.1,
            QueryEmbedding::Unavailable
        ));
    }
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        2,
        "failures are never cached"
    );
    task.abort();
}

#[tokio::test]
async fn query_cache_deduplicates_and_completes_after_initiator_cancels() {
    use takomo::query_embeddings::{QueryCache, QueryEmbedding};
    let calls = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (c, s, r) = (calls.clone(), started.clone(), release.clone());
    let router = axum::Router::new().route(
        "/embeddings",
        axum::routing::post(move || {
            let (c, s, r) = (c.clone(), s.clone(), r.clone());
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                s.notify_one();
                r.notified().await;
                axum::Json(json!({"data":[{"index":0,"embedding":[1.,0.,0.]}]}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = config(&format!(
        "http://{}/embeddings",
        listener.local_addr().unwrap()
    ));
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let cache = QueryCache::new(1, std::time::Duration::from_secs(600));
    let (owned, config) = (cache.clone(), cfg.clone());
    let first = tokio::spawn(async move { owned.get(&config, "key", "user", "query").await });
    started.notified().await;
    assert!(matches!(
        cache.get(&cfg, "key", "user", "another").await.1,
        QueryEmbedding::Throttled
    ));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "pending capacity is bounded"
    );
    first.abort();
    let (owned, config) = (cache.clone(), cfg.clone());
    let second = tokio::spawn(async move { owned.get(&config, "key", "user", "query").await });
    release.notify_one();
    let (old_generation, result) = second.await.unwrap();
    assert!(matches!(result, QueryEmbedding::Ready(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    cache.invalidate();
    let (owned, config) = (cache.clone(), cfg.clone());
    let pending = tokio::spawn(async move { owned.get(&config, "key", "user", "query").await });
    started.notified().await;
    cache.invalidate();
    release.notify_one();
    let (generation, _) = pending.await.unwrap();
    assert!(!cache.is_current(generation));
    assert!(!cache.is_current(old_generation));
    let (owned, config) = (cache.clone(), cfg.clone());
    let final_request =
        tokio::spawn(async move { owned.get(&config, "key", "user", "query").await });
    started.notified().await;
    release.notify_one();
    assert!(cache.is_current(final_request.await.unwrap().0));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "old completion never repopulates new generation"
    );
    task.abort();
}

#[tokio::test]
async fn cached_query_still_authorizes_and_retrieves_current_document() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, node) = fixture(&app).await;
    let (endpoint, calls, task) = mock_provider().await;
    let mut setting = serde_json::to_value(config(&endpoint)).unwrap();
    setting["api_key"] = json!("local-mock");
    assert_eq!(
        app.put(&app.admin, "/v1/settings/embeddings", setting.clone())
            .await
            .0,
        StatusCode::OK
    );
    let manual = format!("/v1/mindmaps/{map}/search/sync");
    app.post(&app.worker, &manual, json!({})).await;
    process_jobs(Arc::new(app.open_store())).await.unwrap();
    let path = format!("/v1/mindmaps/{map}/search?q=payment");
    assert_eq!(app.get(&app.worker, &path).await.1["mode"], "hybrid");
    let before = calls.load(Ordering::SeqCst);
    assert_eq!(app.get(&app.worker, &path).await.1["mode"], "hybrid");
    assert_eq!(calls.load(Ordering::SeqCst), before);
    let forbidden = app.mint("forbidden", &["read"], Some(&["other"]));
    assert_eq!(app.get(&forbidden, &path).await.0, StatusCode::FORBIDDEN);
    assert_eq!(calls.load(Ordering::SeqCst), before);
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({"notes":"New payment passage replaces the old receipt."}),
    )
    .await;
    app.post(&app.worker, &manual, json!({})).await;
    process_jobs(Arc::new(app.open_store())).await.unwrap();
    let reindexed = calls.load(Ordering::SeqCst);
    let (_, body) = app.get(&app.worker, &path).await;
    assert_eq!(body["mode"], "hybrid");
    assert!(
        body["results"][0]["excerpt"]
            .as_str()
            .unwrap()
            .contains("New payment passage"),
        "{body}"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        reindexed,
        "only query vector is cached, never source results"
    );
    assert_eq!(
        app.put(&app.admin, "/v1/settings/embeddings", setting)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(app.get(&app.worker, &path).await.1["mode"], "hybrid");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        reindexed + 1,
        "settings route invalidates cached vectors"
    );
    task.abort();
}

#[tokio::test]
async fn query_cache_inflight_config_change_cannot_return_an_old_generation() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, _) = fixture(&app).await;
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (s, r) = (started.clone(), release.clone());
    let router = axum::Router::new().route(
        "/embeddings",
        axum::routing::post(move || {
            let (s, r) = (s.clone(), r.clone());
            async move {
                s.notify_one();
                r.notified().await;
                axum::Json(json!({"data":[{"index":0,"embedding":[1.,0.,0.]}]}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = config(&format!(
        "http://{}/embeddings",
        listener.local_addr().unwrap()
    ));
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut body = serde_json::to_value(&cfg).unwrap();
    body["api_key"] = json!("key");
    app.put(&app.admin, "/v1/settings/embeddings", body.clone())
        .await;
    let store = app.open_store();
    store
        .refresh_search(&map, true, takomo::ids::now_ms())
        .unwrap();
    let job = store
        .claim_embedding_job(takomo::ids::now_ms())
        .unwrap()
        .unwrap();
    assert!(store
        .finish_embedding_job(&job, Ok(&vec![vec![1., 0., 0.]; job.chunks.len()]))
        .unwrap());
    for disabled in [false, true] {
        let request = app
            .client
            .get(format!("{}/v1/mindmaps/{map}/search?q=Invoices", app.base))
            .bearer_auth(&app.worker);
        let pending =
            tokio::spawn(
                async move { request.send().await.unwrap().json::<Value>().await.unwrap() },
            );
        started.notified().await;
        // Even an otherwise identical full settings save starts a fresh cache generation.
        if disabled {
            body["api_key"] = json!("");
        }
        app.put(&app.admin, "/v1/settings/embeddings", body.clone())
            .await;
        release.notify_one();
        let response = pending.await.unwrap();
        assert_eq!(response["mode"], "keyword", "{response}");
        assert_eq!(
            response["semantic_status"],
            if disabled { "unconfigured" } else { "indexing" }
        );
        assert!(!response["results"].as_array().unwrap().is_empty());
    }
    task.abort();
}
