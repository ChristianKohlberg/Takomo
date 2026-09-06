use reqwest::StatusCode;
use serde_json::json;
mod common;
use common::TestApp;

#[tokio::test]
async fn document_appearance_persists_resets_and_is_project_scoped() {
    let app = TestApp::spawn().await;
    let path = "/v1/projects/tp/document-appearance";
    let default = json!({"template":"balanced","overrides":{}});
    assert_eq!(
        app.get(&app.worker, "/v1/projects").await.1[0]["document_appearance"],
        default
    );
    let selected = json!({"template":"strong","overrides":{"h1_size":31.0,"body_size":17.0,"heading_weight":600.0,"line_height":1.7,"heading_spacing":24.0}});
    assert_eq!(
        app.put(&app.worker, path, selected.clone()).await.0,
        StatusCode::FORBIDDEN
    );
    let response = app.put(&app.admin, path, selected.clone()).await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.1["document_appearance"], selected);
    let store = app.open_store();
    assert_eq!(
        store.get_project("tp").unwrap().unwrap().to_json()["document_appearance"],
        selected
    );
    store
        .create_project("other", "Other", None, "test")
        .unwrap();
    assert_eq!(
        store.get_project("other").unwrap().unwrap().to_json()["document_appearance"],
        default
    );
    let scoped = app.mint("other-admin", &["read", "admin"], Some(&["other"]));
    assert_eq!(
        app.put(&scoped, path, default.clone()).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.put(
            &app.admin,
            "/v1/projects/missing/document-appearance",
            default.clone()
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    for invalid in [
        json!({"template":"custom","overrides":{}}),
        json!({"template":"balanced"}),
        json!({"template":"balanced","overrides":{},"unknown":1}),
        json!({"template":"balanced","overrides":{"h1size":28}}),
        json!({"template":"balanced","overrides":{"h1_size":"28"}}),
    ] {
        assert_eq!(
            app.put(&app.admin, path, invalid).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    for (field, value) in [
        ("h1_size", 65.0),
        ("h2_size", 11.0),
        ("h3_size", 100.0),
        ("body_size", 25.0),
        ("heading_weight", 650.0),
        ("heading_weight", 900.0),
        ("line_height", 0.9),
        ("line_height", 2.6),
        ("heading_spacing", -1.0),
        ("heading_spacing", 49.0),
    ] {
        assert_eq!(
            app.put(
                &app.admin,
                path,
                json!({"template":"balanced","overrides":{field:value}})
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(
        app.get(&app.worker, "/v1/projects")
            .await
            .1
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == "tp")
            .unwrap()["document_appearance"],
        selected
    );
    let reset = app
        .put(
            &app.admin,
            path,
            json!({"template":"balanced","overrides":{"h1_size":null}}),
        )
        .await;
    assert_eq!(reset.0, StatusCode::OK);
    assert_eq!(reset.1["document_appearance"], default);
    store
        .set_project_archived("tp", true, false, "test")
        .unwrap();
    assert_eq!(
        app.put(&app.admin, path, selected).await.0,
        StatusCode::CONFLICT
    );
}

#[test]
fn existing_database_gains_balanced_default_without_changing_project_content() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("existing.db");
    {
        let store = takomo::store::Store::open(&path).unwrap();
        store
            .create_project("old", "Existing project", None, "test")
            .unwrap();
        store
            .set_style_guide("old", Some("Keep this house style"), "test")
            .unwrap();
    }
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "ALTER TABLE projects DROP COLUMN document_appearance_json",
            [],
        )
        .unwrap();
    }
    let store = takomo::store::Store::open(&path).unwrap();
    let project = store.get_project("old").unwrap().unwrap().to_json();
    assert_eq!(project["name"], "Existing project");
    assert_eq!(project["style_guide"], "Keep this house style");
    assert_eq!(
        project["document_appearance"],
        json!({"template":"balanced","overrides":{}})
    );
    let invalid = takomo::store::DocumentAppearance {
        overrides: takomo::store::DocumentAppearanceOverrides {
            h1_size: Some(f64::NAN),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(store
        .set_document_appearance("old", invalid, "test")
        .is_err());
}
