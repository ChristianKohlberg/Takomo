//! Deployment-owned GitHub App. Keys stay on the server; tokens are short-lived.
use crate::error::{ApiError, ApiResult};
use axum::http::StatusCode;
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use serde_json::{json, Value};
use std::time::Duration;

fn error(message: &str) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, "integration.github", message)
}
pub struct Github {
    id: String,
    pub slug: String,
    key: ring::signature::RsaKeyPair,
    client: reqwest::Client,
}
impl Github {
    pub fn configured() -> bool {
        [
            "TAKOMO_GITHUB_APP_ID",
            "TAKOMO_GITHUB_APP_SLUG",
            "TAKOMO_GITHUB_PRIVATE_KEY_FILE",
        ]
        .iter()
        .all(|k| std::env::var(k).is_ok_and(|v| !v.is_empty()))
    }
    pub fn load() -> ApiResult<Self> {
        let env = |key| {
            std::env::var(key).map_err(|_| {
                error("Configure the deployment GitHub App before connecting repositories.")
            })
        };
        let id = env("TAKOMO_GITHUB_APP_ID")?;
        let slug = env("TAKOMO_GITHUB_APP_SLUG")?;
        if !id.bytes().all(|b| b.is_ascii_digit())
            || id.is_empty()
            || slug.is_empty()
            || !slug.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(error(
                "Invalid GitHub App configuration. Ask the operator to check its ID and slug.",
            ));
        }
        let pem =
            std::fs::read_to_string(env("TAKOMO_GITHUB_PRIVATE_KEY_FILE")?).map_err(|_| {
                error("Cannot read the GitHub App private key. Check the configured key file.")
            })?;
        let der = STANDARD
            .decode(
                pem.lines()
                    .filter(|l| !l.starts_with("---"))
                    .collect::<String>(),
            )
            .map_err(|_| error("Invalid GitHub App key encoding."))?;
        let key = if pem.contains("BEGIN RSA PRIVATE KEY") {
            ring::signature::RsaKeyPair::from_der(&der)
        } else {
            ring::signature::RsaKeyPair::from_pkcs8(&der)
        }
        .map_err(|_| error("Invalid GitHub App RSA private key."))?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("takomo-github/0.1")
            .build()
            .map_err(|_| error("Cannot initialize GitHub transport."))?;
        Ok(Self {
            id,
            slug,
            key,
            client,
        })
    }
    fn jwt(&self) -> ApiResult<String> {
        let now = crate::ids::now_ms() / 1000;
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let payload =
            URL_SAFE_NO_PAD.encode(json!({"iat":now-60,"exp":now+540,"iss":self.id}).to_string());
        let input = format!("{header}.{payload}");
        let mut signature = vec![0; self.key.public().modulus_len()];
        self.key
            .sign(
                &ring::signature::RSA_PKCS1_SHA256,
                &ring::rand::SystemRandom::new(),
                input.as_bytes(),
                &mut signature,
            )
            .map_err(|_| error("Cannot sign GitHub App authentication."))?;
        Ok(format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature)))
    }
    async fn request(&self, path: &str, token: &str, body: Option<Value>) -> ApiResult<Value> {
        let mut req = self
            .client
            .request(
                if body.is_some() {
                    reqwest::Method::POST
                } else {
                    reqwest::Method::GET
                },
                format!("https://api.github.com{path}"),
            )
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(body) = body {
            req = req.json(&body);
        }
        let mut response = req.send().await.map_err(|_| {
            error("GitHub could not be reached. Retry without changing repository access.")
        })?;
        if !response.status().is_success() {
            return Err(error("GitHub refused access. Check the installation, selected repositories and Contents read permission, then refresh."));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| error("GitHub response was interrupted. Retry the request."))?
        {
            if bytes.len() + chunk.len() > 2_000_000 {
                return Err(error(
                    "GitHub response exceeded the limit. Use a smaller selection.",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| error("GitHub returned an invalid response."))
    }
    pub async fn installations(&self) -> ApiResult<Value> {
        self.request("/app/installations?per_page=100", &self.jwt()?, None)
            .await
    }
    pub async fn token(&self, installation: u64, repository: Option<u64>) -> ApiResult<String> {
        let mut body = json!({"permissions":{"contents":"read"}});
        if let Some(repository) = repository {
            body["repository_ids"] = json!([repository]);
        }
        let result = self
            .request(
                &format!("/app/installations/{installation}/access_tokens"),
                &self.jwt()?,
                Some(body),
            )
            .await?;
        result["token"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| error("GitHub did not issue an installation token."))
    }
    pub async fn repositories(&self, installation: u64, page: u64) -> ApiResult<Value> {
        self.request(
            &format!("/installation/repositories?per_page=100&page={page}"),
            &self.token(installation, None).await?,
            None,
        )
        .await
    }
    pub async fn revision(
        &self,
        installation: u64,
        repository: u64,
        full_name: &str,
    ) -> ApiResult<String> {
        if !valid_repository(full_name) {
            return Err(error("Choose a valid GitHub repository."));
        }
        let token = self.token(installation, Some(repository)).await?;
        let repo = self
            .request(&format!("/repos/{full_name}"), &token, None)
            .await?;
        if repo["id"].as_u64() != Some(repository) {
            return Err(error(
                "Repository identity changed. Refresh the repository list.",
            ));
        }
        let branch = repo["default_branch"]
            .as_str()
            .ok_or_else(|| error("Repository has no default branch."))?;
        let mut url = reqwest::Url::parse("https://api.github.com/").unwrap();
        url.path_segments_mut().unwrap().push(branch);
        let encoded = url.path().trim_start_matches('/');
        let commit = self
            .request(
                &format!("/repos/{full_name}/commits/{encoded}"),
                &token,
                None,
            )
            .await?;
        commit["sha"]
            .as_str()
            .filter(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(str::to_owned)
            .ok_or_else(|| error("Repository has no readable commit."))
    }
}
pub fn valid_repository(name: &str) -> bool {
    let parts: Vec<_> = name.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|p| {
            !p.is_empty()
                && *p != "."
                && *p != ".."
                && p.len() <= 100
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
}

/// Account-specific URL derived from verified installation metadata, never caller input.
pub fn installation_management_url(installation: &Value) -> ApiResult<String> {
    let id = installation["id"]
        .as_u64()
        .ok_or_else(|| error("Invalid GitHub installation ID."))?;
    match installation["account"]["type"].as_str() {
        Some("User") => Ok(format!("https://github.com/settings/installations/{id}")),
        Some("Organization") => {
            let account = installation["account"]["login"]
                .as_str()
                .filter(|s| {
                    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                })
                .ok_or_else(|| error("Invalid GitHub organization name."))?;
            Ok(format!(
                "https://github.com/organizations/{account}/settings/installations/{id}"
            ))
        }
        _ => Err(error(
            "Only personal and organization GitHub installations are supported.",
        )),
    }
}
