//! Two ways the specification could be lost in one move, and the guards on them.
//!
//! - `PATCH …/nodes/{node}` with `notes` rewrites a section's prose as plain
//!   paragraphs. On a section with structure that would flatten it, so it is
//!   refused (`conflict.notes_would_flatten`) and nothing changes — while empty,
//!   placeholder and plain sections keep working, because that is how
//!   placeholders are seeded.
//! - `DELETE /v1/mindmaps/{id}` removes the whole specification, so it needs the
//!   admin scope and the id repeated as `confirm_id`, like a reset.
mod common;

use common::TestApp;
use reqwest::StatusCode;
use serde_json::{json, Value};
use yrs::types::Attrs;
use yrs::updates::decoder::Decode;
use yrs::{
    Any, Doc, Map, Out, ReadTxn, Text, Transact, Update, Xml, XmlElementPrelim, XmlFragment,
    XmlOut, XmlTextPrelim,
};

async fn map_with_section(app: &TestApp, notes: Option<&str>) -> (String, String) {
    let (status, created) = app
        .post(
            &app.admin,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Specification"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let map = created["mindmap"]["id"].as_str().unwrap().to_string();
    let mut node = json!({"text":"Payments"});
    if let Some(notes) = notes {
        node["notes"] = json!(notes);
    }
    let (status, added) = app
        .post(&app.admin, &format!("/v1/mindmaps/{map}/nodes"), node)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{added}");
    let node = added["nodes"][0]["id"].as_str().unwrap().to_string();
    (map, node)
}

fn persisted(app: &TestApp, id: &str) -> Doc {
    let doc = Doc::new();
    for update in app.open_store().load_collab_updates(id).unwrap() {
        doc.transact_mut()
            .apply_update(Update::decode_v1(&update).unwrap())
            .unwrap();
    }
    doc
}

/// The section's prose fragment, read the way the document view binds it.
fn prose(doc: &Doc, node: &str) -> yrs::XmlFragmentRef {
    let nodes = doc.get_or_insert_map("nodes");
    let txn = doc.transact();
    let Some(Out::YMap(entry)) = nodes.get(&txn, node) else {
        panic!("no node {node}");
    };
    let Some(Out::YXmlFragment(frag)) = entry.get(&txn, "prose") else {
        panic!("node {node} has no prose");
    };
    frag
}

/// Give the section what the document view writes: a heading, and a paragraph
/// with a bold word in it — appended to the persisted log as a peer would.
fn make_rich(app: &TestApp, map: &str, node: &str) {
    let doc = persisted(app, map);
    let before = doc.transact().state_vector();
    let frag = prose(&doc, node);
    {
        let mut txn = doc.transact_mut();
        let heading = frag.push_back(&mut txn, XmlElementPrelim::empty("heading"));
        heading.insert_attribute(&mut txn, "id", "blk_head01");
        heading.insert_attribute(&mut txn, "level", "3");
        heading.push_back(&mut txn, XmlTextPrelim::new("Refunds"));
        let paragraph = frag.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        paragraph.insert_attribute(&mut txn, "id", "blk_bold01");
        let text = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
        let mut bold = Attrs::new();
        bold.insert("bold".into(), Any::Map(Default::default()));
        text.insert_with_attributes(&mut txn, 0, "Never partial.", bold);
    }
    let update = doc.transact().encode_diff_v1(&before);
    app.open_store()
        .append_collab_update(map, &update, "test")
        .unwrap();
}

fn tags(doc: &Doc, node: &str) -> Vec<String> {
    let frag = prose(doc, node);
    let txn = doc.transact();
    frag.children(&txn)
        .filter_map(|child| match child {
            XmlOut::Element(el) => Some(el.tag().to_string()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn notes_never_flatten_a_structured_section() {
    let app = TestApp::spawn().await;
    let (map, node) = map_with_section(&app, Some("The lead paragraph.")).await;
    make_rich(&app, &map, &node);
    let before = tags(&persisted(&app, &map), &node);
    assert_eq!(before, ["paragraph", "heading", "paragraph"]);

    let path = format!("/v1/mindmaps/{map}/nodes/{node}");
    let (status, refused) = app
        .patch(&app.worker, &path, json!({"notes":"Flattened."}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "conflict.notes_would_flatten", "{refused}");
    assert!(
        refused["remedy"]
            .as_str()
            .unwrap()
            .contains("takomo_plan_propose"),
        "the refusal points at proposals: {refused}"
    );

    // Refused as a whole: a title sent alongside does not land either.
    let (status, _) = app
        .patch(
            &app.worker,
            &path,
            json!({"notes":"Flattened.","text":"Renamed"}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let after = persisted(&app, &map);
    assert_eq!(tags(&after, &node), before, "the structure is intact");
    let (_, read) = app.get(&app.admin, &format!("/v1/mindmaps/{map}")).await;
    let section: &Value = read["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == node.as_str())
        .unwrap();
    assert_eq!(section["text"], "Payments", "{section}");
    // The plain reading still holds all three blocks (how a marked run reads
    // as plain text is `prose::plain_text`'s business, not this test's).
    let notes = section["notes"].as_str().unwrap();
    assert!(
        notes.starts_with("The lead paragraph.\nRefunds\n") && notes.contains("Never partial."),
        "{section}"
    );

    // Everything else about the section still changes.
    let (status, renamed) = app
        .patch(&app.worker, &path, json!({"text":"Payments and refunds"}))
        .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
}

#[tokio::test]
async fn notes_still_seed_and_rewrite_placeholder_and_plain_sections() {
    let app = TestApp::spawn().await;
    // A fresh section with no prose at all: how a placeholder is seeded.
    let (map, node) = map_with_section(&app, None).await;
    let path = format!("/v1/mindmaps/{map}/nodes/{node}");
    let placeholder = "Wird befüllt – Entwurf folgt als Vorschlag.";
    let (status, seeded) = app
        .patch(&app.worker, &path, json!({"notes": placeholder}))
        .await;
    assert_eq!(status, StatusCode::OK, "{seeded}");
    assert_eq!(seeded["node"]["notes"], placeholder);

    // Plain paragraphs over plain paragraphs loses nothing, so it stays allowed.
    let (status, rewritten) = app
        .patch(&app.worker, &path, json!({"notes":"First.\nSecond."}))
        .await;
    assert_eq!(status, StatusCode::OK, "{rewritten}");
    assert_eq!(rewritten["node"]["notes"], "First.\nSecond.");
    assert_eq!(
        tags(&persisted(&app, &map), &node),
        ["paragraph", "paragraph"]
    );
}

#[tokio::test]
async fn deleting_the_specification_needs_admin_and_the_id_confirmed() {
    let app = TestApp::spawn().await;
    let (map, _) = map_with_section(&app, Some("Keep me.")).await;
    let path = format!("/v1/mindmaps/{map}");

    // Write scope is not enough, confirmed or not.
    for token in [&app.worker, &app.human] {
        let (status, body) = app
            .delete_with(token, &path, json!({"confirm_id": map}))
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    }
    let limited = app.mint("limited", &["read", "write", "admin"], Some(&["other"]));
    assert_eq!(
        app.delete_with(&limited, &path, json!({"confirm_id": map}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    // Admin, but unconfirmed: no body at all, a body without the id, a wrong id.
    let (status, bare) = app.delete(&app.admin, &path).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bare}");
    assert_eq!(bare["code"], "validation.field_required", "{bare}");
    assert!(
        bare["remedy"].as_str().unwrap().contains("confirm_id"),
        "{bare}"
    );
    assert_eq!(
        app.delete_with(&app.admin, &path, json!({})).await.0,
        StatusCode::BAD_REQUEST
    );
    let (status, wrong) = app
        .delete_with(&app.admin, &path, json!({"confirm_id":"mm-other"}))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{wrong}");
    assert_eq!(wrong["code"], "validation.confirm_id", "{wrong}");
    assert_eq!(
        app.delete_with(&app.admin, &path, json!({"confirm_id": map, "force": true}))
            .await
            .0,
        StatusCode::BAD_REQUEST,
        "unknown fields are refused"
    );

    // Nothing above removed anything.
    let (status, still) = app.get(&app.admin, &path).await;
    assert_eq!(status, StatusCode::OK, "{still}");
    assert_eq!(still["nodes"].as_array().unwrap().len(), 1);

    // Archived projects are frozen for this too.
    app.post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;
    assert_eq!(
        app.delete_with(&app.admin, &path, json!({"confirm_id": map}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    app.post(&app.admin, "/v1/projects/tp/unarchive", json!({}))
        .await;

    let (status, gone) = app
        .delete_with(&app.admin, &path, json!({"confirm_id": map}))
        .await;
    assert_eq!(status, StatusCode::OK, "{gone}");
    assert_eq!(gone["removed_nodes"], 1, "{gone}");
    assert_eq!(app.get(&app.admin, &path).await.0, StatusCode::NOT_FOUND);
}
