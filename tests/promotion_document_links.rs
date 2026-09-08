mod common;
use common::TestApp;
use reqwest::StatusCode;
use rusqlite::Connection;
use serde_json::{json, Value};
use takomo::store::{mindmapdoc, BranchPromotion};

async fn map(app: &TestApp) -> String {
    let (status, result) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Promotion sources"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{result}");
    result["mindmap"]["id"].as_str().unwrap().to_owned()
}
fn source(app: &TestApp, ticket: &str) -> (String, String, String, String, String) {
    Connection::open(app.db_path()).unwrap().query_row(
        "SELECT section_id,relation,provenance,state,section_version FROM ticket_document_links WHERE ticket=?1",
        [ticket], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
    ).unwrap()
}

#[tokio::test]
async fn promoted_epic_and_children_keep_exact_independent_source_sections() {
    let app = TestApp::spawn_without_sweeper().await;
    let map = map(&app).await;
    let (_, root) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({"text":"Payments"}),
        )
        .await;
    let root = root["nodes"][0]["id"].as_str().unwrap().to_owned();
    let (_, children) = app.post(&app.worker, &format!("/v1/mindmaps/{map}/nodes"), json!({"nodes":[{"parent":root,"text":"Same title","notes":"First exact requirement"},{"parent":root,"text":"Same title","notes":"Second exact requirement"}]})).await;
    let ids: Vec<String> = children["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["id"].as_str().unwrap().to_owned())
        .collect();
    let (status, result) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{root}/promote"),
            json!({"target":"epic"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{result}");
    let epic = result["created"]["id"].as_str().unwrap();
    assert_eq!(source(&app, epic).0, root);
    for (ticket, section) in result["created"]["children"]
        .as_array()
        .unwrap()
        .iter()
        .zip(&ids)
    {
        let link = source(&app, ticket.as_str().unwrap());
        assert_eq!(&link.0, section);
        assert_eq!(
            (&link.1, &link.2, &link.3),
            (
                &"source".to_owned(),
                &"direct".to_owned(),
                &"accepted".to_owned()
            )
        );
        assert_eq!(link.4.len(), 64);
    }
    let (_, current) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    let root_node = current["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == root)
        .unwrap();
    assert_eq!(root_node["promoted"]["id"], epic);
    for id in ids {
        let child = current["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == id)
            .unwrap();
        assert!(
            child["promoted"].is_null(),
            "child's existing promotion field must not be repurposed: {child}"
        );
    }
}

#[tokio::test]
async fn missing_promotion_section_creates_no_ticket_or_source_link() {
    let app = TestApp::spawn_without_sweeper().await;
    let map = map(&app).await;
    let (status, _) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/missing/promote"),
            json!({"target":"epic"}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let connection = Connection::open(app.db_path()).unwrap();
    let tickets: i64 = connection
        .query_row("SELECT COUNT(*) FROM tickets", [], |row| row.get(0))
        .unwrap();
    let links: i64 = connection
        .query_row("SELECT COUNT(*) FROM ticket_document_links", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!((tickets, links), (0, 0));
}

#[tokio::test]
async fn direct_promotion_works_when_unrelated_document_content_exceeds_classification_limit() {
    use yrs::{Transact, XmlFragment, XmlTextPrelim};
    let app = TestApp::spawn_without_sweeper().await;
    let map = map(&app).await;
    let store = app.open_store();
    let (root, promoted) = store
        .edit_mindmap_document(&map, |doc| {
            let added = mindmapdoc::add_nodes(
                doc,
                &[
                    mindmapdoc::NodeAdd {
                        title: "Small branch".into(),
                        ..Default::default()
                    },
                    mindmapdoc::NodeAdd {
                        title: "Unrelated large prose".into(),
                        ..Default::default()
                    },
                ],
                "test",
            )?;
            let root = &added[0].0;
            let fragment = mindmapdoc::section_prose(doc, &added[1].0)?;
            fragment.push_back(
                &mut doc.transact_mut(),
                XmlTextPrelim::new("x".repeat(8_000_001)),
            );
            assert!(
                takomo::store::ticket_document::document_from_doc(doc, &map, "Large document")?
                    .to_string()
                    .len()
                    > 8_000_000
            );
            let captured = BranchPromotion::capture_source(doc, &map, &[root])?;
            assert!(captured.to_string().len() < 1000);
            let created = store.promote_branch(
                &BranchPromotion {
                    map_id: &map,
                    node_id: root,
                    target: "epic",
                    title: "Small branch",
                    branch_outline: "Small branch",
                    children: &[],
                    source_document: &captured,
                },
                "test",
            )?;
            mindmapdoc::set_promoted(doc, root, "epic", created["id"].as_str().unwrap())?;
            Ok((root.clone(), created))
        })
        .unwrap();
    assert_eq!(source(&app, promoted["id"].as_str().unwrap()).0, root);
}

#[tokio::test]
async fn invalid_direct_source_rolls_back_the_entire_promotion() {
    let app = TestApp::spawn_without_sweeper().await;
    let map = map(&app).await;
    let store = app.open_store();
    let invalid: Value = json!({"mindmap_id":map,"sections":[]});
    assert!(store
        .promote_branch(
            &BranchPromotion {
                map_id: &map,
                node_id: "missing",
                target: "epic",
                title: "No partial ticket",
                branch_outline: "Invalid source",
                children: &[],
                source_document: &invalid
            },
            "test"
        )
        .is_err());
    let count: i64 = Connection::open(app.db_path())
        .unwrap()
        .query_row("SELECT COUNT(*) FROM tickets", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}
