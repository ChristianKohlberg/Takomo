use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::post,
    Router,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use takomo::{
    diagrams::DiagramRenderer,
    server::{build_router, AppState},
    store::Store,
};

const SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\"><text>preview</text></svg>";
async fn mock(
    State(calls): State<Arc<AtomicUsize>>,
    Path(engine): Path<String>,
    source: String,
) -> impl IntoResponse {
    calls.fetch_add(1, Ordering::SeqCst);
    assert!(matches!(engine.as_str(), "mermaid" | "plantuml" | "d2"));
    let (status, body) = match source.as_str() {
        "slow" => {
            tokio::time::sleep(std::time::Duration::from_secs(11)).await;
            (StatusCode::OK, SVG.into())
        }
        "syntax" => (StatusCode::BAD_REQUEST, "secret upstream internals".into()),
        "failure" => (StatusCode::INTERNAL_SERVER_ERROR, "secret source".into()),
        "external" => (StatusCode::OK, "<svg xmlns=\"http://www.w3.org/2000/svg\"><image href=\"h&#116;tp://private/secret\"/></svg>".into()),
        "malformed" => (StatusCode::OK, "<svg><g></svg>".into()),
        "multiple-roots" => (StatusCode::OK, "<svg/><svg/>".into()),
        "invalid" => (StatusCode::OK, "<html>bad result</html>".into()),
        "large" => (StatusCode::OK, "x".repeat(2 * 1024 * 1024 + 1)),
        "redirect" => (StatusCode::TEMPORARY_REDIRECT, String::new()),
        _ => (StatusCode::OK, SVG.into()),
    };
    (
        status,
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::LOCATION, "/mermaid/svg"),
        ],
        body,
    )
}
struct App {
    base: String,
    reader: String,
    other: String,
    writer: String,
    calls: Arc<AtomicUsize>,
    _tmp: tempfile::TempDir,
}
impl App {
    async fn new(configured: bool) -> Self {
        let calls = Arc::new(AtomicUsize::new(0));
        let mock_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let renderer = DiagramRenderer::new(
            &format!("http://{}", mock_listener.local_addr().unwrap()),
            "test-v1",
        )
        .unwrap();
        let mock_router = Router::new()
            .route("/{engine}/svg", post(mock))
            .with_state(calls.clone());
        tokio::spawn(async move {
            axum::serve(mock_listener, mock_router).await.unwrap();
        });
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path().join("test.db")).unwrap();
        store
            .create_project("tp", "Test", None, "test:setup")
            .unwrap();
        let token = |scopes: Vec<String>, projects: Vec<String>| {
            store
                .create_token("test:reader", &scopes, Some(&projects), 10_000, None, None)
                .unwrap()
                .1
        };
        let reader = token(vec!["read".into()], vec!["tp".into()]);
        let other = token(vec!["read".into()], vec!["other".into()]);
        let writer = token(vec!["write".into()], vec!["tp".into()]);
        let mut state = Arc::try_unwrap(AppState::new(store))
            .map_err(|_| ())
            .unwrap();
        state.diagrams = configured.then_some(renderer);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router = build_router(Arc::new(state));
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            base,
            reader,
            other,
            writer,
            calls,
            _tmp: tmp,
        }
    }
    async fn render(&self, token: &str, body: Value) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{}/v1/diagrams/render", self.base))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .unwrap()
    }
}
fn body(engine: &str, source: &str) -> Value {
    json!({"project":"tp", "engine":engine, "source":source})
}

#[tokio::test]
async fn diagrams_authorize_before_cache_and_render_all_engines() {
    let app = App::new(true).await;
    for engine in ["mermaid", "plantuml", "d2"] {
        for _ in 0..2 {
            let r = app.render(&app.reader, body(engine, "hello")).await;
            assert_eq!(r.status(), StatusCode::OK);
            assert_eq!(r.json::<Value>().await.unwrap()["svg"], SVG);
        }
    }
    assert_eq!(app.calls.load(Ordering::SeqCst), 3);
    for (token, status) in [
        ("", StatusCode::UNAUTHORIZED),
        (app.other.as_str(), StatusCode::FORBIDDEN),
        (app.writer.as_str(), StatusCode::FORBIDDEN),
    ] {
        assert_eq!(
            app.render(token, body("mermaid", "hello")).await.status(),
            status
        );
    }
    assert_eq!(app.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn diagrams_bound_and_validate_inputs_and_disable_cleanly() {
    let app = App::new(true).await;
    for input in [
        body("../../evil", "hello"),
        body("d2", "...@secret"),
        body("d2", "x: a@b"),
        body("mermaid", " "),
        body("d2", &"x".repeat(50_000 + 1)),
        json!({"project":"tp", "engine":"d2", "source":"x", "url":"http://evil"}),
    ] {
        assert_eq!(
            app.render(&app.reader, input).await.status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(app.calls.load(Ordering::SeqCst), 0);
    let disabled = App::new(false).await;
    let r = disabled.render(&disabled.reader, body("d2", "x")).await;
    assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        r.json::<Value>().await.unwrap()["code"],
        "diagram.not_configured"
    );
}

#[tokio::test]
async fn diagrams_reject_upstream_failures_without_leaks_or_caching() {
    let app = App::new(true).await;
    for (source, status) in [
        ("syntax", StatusCode::UNPROCESSABLE_ENTITY),
        ("failure", StatusCode::BAD_GATEWAY),
        ("invalid", StatusCode::BAD_GATEWAY),
        ("malformed", StatusCode::BAD_GATEWAY),
        ("multiple-roots", StatusCode::BAD_GATEWAY),
        ("large", StatusCode::UNPROCESSABLE_ENTITY),
        ("redirect", StatusCode::BAD_GATEWAY),
    ] {
        for _ in 0..2 {
            let r = app.render(&app.reader, body("plantuml", source)).await;
            assert_eq!(r.status(), status, "{source}");
            assert!(!r.text().await.unwrap().contains("secret"));
        }
    }
    assert_eq!(app.calls.load(Ordering::SeqCst), 14);
    let r = app.render(&app.reader, body("d2", "external")).await;
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        r.json::<Value>().await.unwrap()["code"],
        "validation.diagram_assets"
    );
}

#[tokio::test]
async fn diagrams_limit_concurrency_and_timeout_then_release_slots() {
    let app = Arc::new(App::new(true).await);
    let mut pending = Vec::new();
    for _ in 0..4 {
        let app = app.clone();
        pending.push(tokio::spawn(async move {
            app.render(&app.reader, body("d2", "slow")).await.status()
        }));
    }
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while app.calls.load(Ordering::SeqCst) < 4 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        app.render(&app.reader, body("d2", "next")).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    for request in pending {
        assert_eq!(request.await.unwrap(), StatusCode::GATEWAY_TIMEOUT);
    }
    assert_eq!(
        app.render(&app.reader, body("d2", "next")).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn diagrams_evict_oldest_successful_previews() {
    let app = App::new(true).await;
    for i in 0..129 {
        assert_eq!(
            app.render(&app.reader, body("d2", &format!("diagram {i}")))
                .await
                .status(),
            StatusCode::OK
        );
    }
    assert_eq!(app.calls.load(Ordering::SeqCst), 129);
    assert_eq!(
        app.render(&app.reader, body("d2", "diagram 128"))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(app.calls.load(Ordering::SeqCst), 129);
    assert_eq!(
        app.render(&app.reader, body("d2", "diagram 0"))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(app.calls.load(Ordering::SeqCst), 130);
}
