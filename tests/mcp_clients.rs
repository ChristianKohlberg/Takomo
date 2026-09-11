//! Real Node MCP clients against isolated Rust test servers. Opt-in locally
//! because a normal cargo test must remain usable without Node dependencies.
mod common;
use common::TestApp;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires npm ci && npm run build in clients/mcp; required by MCP CI"]
async fn mcp_clients_execute_lifecycle_and_parity_scenarios() {
    for script in ["legacy.mjs", "e2e.mjs", "parity.mjs"] {
        let app = TestApp::spawn().await;
        let mut child = tokio::process::Command::new("node");
        child
            .arg(format!(
                "{}/clients/mcp/test/{script}",
                env!("CARGO_MANIFEST_DIR")
            ))
            .env("TAKOMO_URL", format!("{}/v1", app.base))
            .env("TAKOMO_TOKEN", &app.admin)
            .env("TAKOMO_TEST_PROJECT", "tp")
            .env("TAKOMO_WORKER_TOKEN", &app.worker)
            .env("TAKOMO_OTHER_TOKEN", &app.worker2)
            .kill_on_drop(true);
        let output = tokio::time::timeout(std::time::Duration::from_secs(120), child.output())
            .await
            .expect("MCP client suite timed out")
            .expect("Node starts");
        assert!(
            output.status.success(),
            "{script} failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
