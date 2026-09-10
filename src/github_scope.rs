//! Bounded metadata-only validation of literal GitHub source paths.
use crate::error::{ApiError, ApiResult};
use serde_json::Value;
use std::{collections::HashMap, future::Future};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::validation("validation.github", message.into())
}

/// Fetches only ancestor trees, never blobs or recursive repository inventories.
/// The caller supplies GitHub's verified root tree and authenticated tree reader.
pub async fn verify_paths<F, Fut>(root: &str, include: &[String], mut fetch: F) -> ApiResult<()>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = ApiResult<Value>>,
{
    let mut cache = HashMap::<String, Value>::new();
    for path in include {
        let mut sha = root.to_owned();
        let parts: Vec<_> = if path == "." {
            vec![]
        } else {
            path.split('/').collect()
        };
        for (index, part) in parts.iter().enumerate() {
            if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(invalid(
                    "GitHub returned an invalid tree identity. Refresh and retry.",
                ));
            }
            if !cache.contains_key(&sha) {
                if cache.len() >= 16 {
                    return Err(invalid("Checking this scope requires too many directories. Select fewer or shallower paths."));
                }
                cache.insert(sha.clone(), fetch(sha.clone()).await?);
            }
            let tree = &cache[&sha];
            if tree["truncated"] != false {
                return Err(invalid("GitHub returned an incomplete directory listing. This scope could not be verified."));
            }
            let entries = tree["tree"]
                .as_array()
                .ok_or_else(|| invalid("GitHub returned an invalid directory listing."))?;
            let entry = entries.iter().find(|entry| entry["path"].as_str() == Some(*part))
                .ok_or_else(|| invalid(format!("Source path '{path}' was not found on the selected repository's default branch. Check spelling and spaces.")))?;
            let folder = entry["type"] == "tree" && entry["mode"] == "040000";
            let file =
                entry["type"] == "blob" && (entry["mode"] == "100644" || entry["mode"] == "100755");
            if (!folder && !file) || (index + 1 < parts.len() && !folder) {
                return Err(invalid(format!("Source path '{path}' must be a regular file or folder. Symlinks and submodules cannot be extracted.")));
            }
            if folder {
                sha = entry["sha"]
                    .as_str()
                    .ok_or_else(|| invalid("GitHub returned a folder without a tree identity."))?
                    .into();
            }
        }
    }
    Ok(())
}
