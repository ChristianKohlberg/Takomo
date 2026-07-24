//! Outbound notifications for the "ask a human" board.
//!
//! When an agent raises a question, a human only helps if they find out. The
//! in-app board badge covers the ambient case; this module adds push. It is
//! **off unless configured** — with no `TAKOMO_NOTIFY` set, nothing here runs
//! and the default deploy stays secret-free, honouring the single-binary ethos.
//!
//! Configuration (`TAKOMO_NOTIFY`) is a JSON array of routes, each mapping an
//! expertise tag to a transport + target:
//!
//! ```json
//! [
//!   { "expertise": "domain:billing", "transport": "slack",   "target": "https://hooks.slack.com/services/..." },
//!   { "expertise": "domain:legal",   "transport": "email",   "target": "legal@acme.example" },
//!   { "expertise": "*",              "transport": "webhook", "target": "https://ops.acme.example/takomo" }
//! ]
//! ```
//!
//! A question matches a route when the route's `expertise` is `"*"` or is one of
//! the question's tags (a question with no tags matches only `"*"` routes).
//! Email uses SMTP via `TAKOMO_SMTP_URL` (e.g. `smtps://user:pass@host:465`)
//! and `TAKOMO_SMTP_FROM`. Dispatch is fire-and-forget: it never blocks or fails
//! the API call, and errors are logged to stderr.

use crate::server::AppState;
use crate::store::Question;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize)]
struct Route {
    /// Expertise tag to match, or "*" for every question.
    expertise: String,
    /// slack | webhook | email.
    transport: String,
    /// Slack/webhook URL, or an email recipient.
    target: String,
}

fn load_routes() -> Vec<Route> {
    let raw = match std::env::var("TAKOMO_NOTIFY") {
        Ok(s) if !s.trim().is_empty() => s,
        _ => return Vec::new(),
    };
    match serde_json::from_str::<Vec<Route>>(&raw) {
        Ok(routes) => routes,
        Err(e) => {
            eprintln!(
                "TAKOMO_NOTIFY is set but not valid JSON ([{{expertise,transport,target}}]): {e}"
            );
            Vec::new()
        }
    }
}

fn route_matches(route: &Route, q: &Question) -> bool {
    route.expertise == "*" || q.expertise.iter().any(|t| t == &route.expertise)
}

/// Notify every configured route that matches this question. Non-blocking:
/// spawns background dispatch tasks and returns immediately.
pub fn question_asked(_state: &Arc<AppState>, q: &Question) {
    let routes = load_routes();
    if routes.is_empty() {
        return;
    }
    for route in routes.into_iter().filter(|r| route_matches(r, q)) {
        let q = q.clone();
        tokio::spawn(async move {
            if let Err(e) = dispatch(&route, &q).await {
                // NEVER log route.target — a Slack webhook URL and an SMTP URL
                // are themselves secrets. Log a redacted host only.
                eprintln!(
                    "notify: {} -> {} failed for question {}: {e}",
                    route.transport,
                    redact_target(&route.target),
                    q.id
                );
            }
        });
    }
}

/// Reduce a target to a non-secret hint: the host only, dropping the path/query
/// and any userinfo (a Slack webhook URL and an SMTP URL carry their secret in
/// the path/credentials, so those must never be logged).
fn redact_target(target: &str) -> String {
    if let Some(rest) = target.split("://").nth(1) {
        let authority = rest.split(['/', '?']).next().unwrap_or(rest);
        let host_only = authority.rsplit('@').next().unwrap_or(authority);
        return format!("<{host_only}>");
    }
    // Bare email address: keep only the domain.
    match target.rsplit_once('@') {
        Some((_, domain)) => format!("<{domain}>"),
        None => "<redacted>".to_string(),
    }
}

async fn dispatch(route: &Route, q: &Question) -> Result<(), String> {
    match route.transport.as_str() {
        "slack" => post_json(&route.target, &json!({ "text": slack_text(q) })).await,
        "webhook" => {
            let mut payload = q.to_json();
            payload["event"] = json!("question_asked");
            payload["text"] = json!(plain_text(q));
            post_json(&route.target, &payload).await
        }
        "email" => send_email(&route.target, q).await,
        other => Err(format!("unknown transport '{other}'")),
    }
}

async fn post_json(url: &str, payload: &serde_json::Value) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(url)
        .json(payload)
        .send()
        .await
        // without_url() strips the (secret) target URL reqwest embeds in its error.
        .map_err(|e| e.without_url().to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

async fn send_email(to: &str, q: &Question) -> Result<(), String> {
    use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

    let smtp_url =
        std::env::var("TAKOMO_SMTP_URL").map_err(|_| "TAKOMO_SMTP_URL is not set".to_string())?;
    let from =
        std::env::var("TAKOMO_SMTP_FROM").map_err(|_| "TAKOMO_SMTP_FROM is not set".to_string())?;

    // Strip control chars (incl. CR/LF) from the subject as defense-in-depth
    // against header injection via the agent-authored title.
    let subject: String = format!("[takomo] {} question: {}", q.urgency, q.title)
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    let email = Message::builder()
        .from(
            from.parse()
                .map_err(|e| format!("bad TAKOMO_SMTP_FROM: {e}"))?,
        )
        .to(to
            .parse()
            .map_err(|e| format!("bad recipient '{to}': {e}"))?)
        .subject(subject)
        .body(plain_text(q))
        .map_err(|e| e.to_string())?;

    let mailer: AsyncSmtpTransport<Tokio1Executor> =
        AsyncSmtpTransport::<Tokio1Executor>::from_url(&smtp_url)
            // Do not interpolate the error: TAKOMO_SMTP_URL carries credentials.
            .map_err(|_| "bad TAKOMO_SMTP_URL (could not parse)".to_string())?
            .timeout(Some(std::time::Duration::from_secs(10)))
            .build();
    mailer.send(email).await.map_err(|e| e.to_string())?;
    Ok(())
}

fn board_link(q: &Question) -> String {
    match std::env::var("TAKOMO_PUBLIC_URL") {
        Ok(base) if !base.trim().is_empty() => {
            format!("{}/board", base.trim_end_matches('/'))
        }
        _ => format!("POST /v1/questions/{}/answer", q.id),
    }
}

fn plain_text(q: &Question) -> String {
    let mut lines = vec![
        format!("{} needs a human decision ({}).", q.ticket, q.kind),
        String::new(),
        q.title.clone(),
    ];
    if !q.body.trim().is_empty() {
        lines.push(String::new());
        lines.push(q.body.clone());
    }
    if !q.options.is_empty() {
        lines.push(String::new());
        lines.push(format!("Options: {}", q.options.join(" · ")));
    }
    if !q.expertise.is_empty() {
        lines.push(format!("For: {}", q.expertise.join(", ")));
    }
    lines.push(String::new());
    lines.push(format!("Answer: {}", board_link(q)));
    lines.join("\n")
}

fn slack_text(q: &Question) -> String {
    format!(
        ":raising_hand: *{} question on `{}`* ({})\n{}\n{}",
        q.urgency,
        q.ticket,
        q.kind,
        q.title,
        board_link(q)
    )
}
