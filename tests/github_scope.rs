use serde_json::{json, Value};
use std::{cell::RefCell, collections::HashMap};
use takomo::github_scope::verify_paths;

fn tree(entries: Value) -> Value {
    json!({"truncated":false,"tree":entries})
}
fn entry(path: &str, kind: &str, mode: &str, sha: &str) -> Value {
    json!({"path":path,"type":kind,"mode":mode,"sha":sha})
}

#[tokio::test]
async fn shared_ancestors_are_cached_and_unselected_subtrees_are_not_read() {
    let root = "a".repeat(40);
    let src = "b".repeat(40);
    let outside = "c".repeat(40);
    let trees = HashMap::from([
        (
            root.clone(),
            tree(json!([
                entry("src", "tree", "040000", &src),
                entry("private", "tree", "040000", &outside)
            ])),
        ),
        (
            src.clone(),
            tree(json!([
                entry("checkout.mjs", "blob", "100644", &outside),
                entry("cli", "blob", "100755", &outside)
            ])),
        ),
    ]);
    let calls = RefCell::new(vec![]);
    verify_paths(
        &root,
        &["src/checkout.mjs".into(), "src/cli".into(), "src".into()],
        |sha| {
            calls.borrow_mut().push(sha.clone());
            std::future::ready(Ok(trees[&sha].clone()))
        },
    )
    .await
    .unwrap();
    assert_eq!(*calls.borrow(), vec![root, src]);
}

#[tokio::test]
async fn missing_paths_and_nonregular_entries_are_rejected() {
    let sha = "a".repeat(40);
    let listing = tree(json!([
        entry("file name.js", "blob", "100644", &sha),
        entry("link", "blob", "120000", &sha),
        entry("module", "commit", "160000", &sha)
    ]));
    for path in [
        "missing",
        "file name.js ",
        "link",
        "module",
        "file name.js/child",
    ] {
        let result = verify_paths(&sha, &[path.into()], |_| {
            std::future::ready(Ok(listing.clone()))
        })
        .await;
        assert!(result.is_err(), "{path}");
    }
    // Spaces within real filenames remain literal; no automatic trimming/rewriting.
    verify_paths(&sha, &["file name.js".into()], |_| {
        std::future::ready(Ok(listing.clone()))
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn incomplete_or_malformed_metadata_never_reports_a_verified_path() {
    for listing in [
        json!({"truncated":true,"tree":[]}),
        json!({"tree":[]}),
        json!({"truncated":false}),
    ] {
        assert!(
            verify_paths(&"a".repeat(40), &["src".into()], |_| std::future::ready(
                Ok(listing.clone())
            ))
            .await
            .is_err()
        );
    }
    let listing = tree(json!([entry("src", "tree", "040000", "../../untrusted")]));
    let calls = RefCell::new(0);
    assert!(verify_paths(&"a".repeat(40), &["src/file".into()], |_| {
        *calls.borrow_mut() += 1;
        std::future::ready(Ok(listing.clone()))
    })
    .await
    .is_err());
    assert_eq!(
        *calls.borrow(),
        1,
        "invalid metadata must never become a request path"
    );
}

#[tokio::test]
async fn deep_paths_stop_at_the_metadata_request_budget() {
    let calls = RefCell::new(0);
    let path = vec!["dir"; 18].join("/");
    assert!(verify_paths(&format!("{:040x}", 0), &[path], |_| {
        *calls.borrow_mut() += 1;
        let next = format!("{:040x}", *calls.borrow());
        std::future::ready(Ok(tree(json!([entry("dir", "tree", "040000", &next)]))))
    })
    .await
    .is_err());
    assert_eq!(*calls.borrow(), 16);
}
