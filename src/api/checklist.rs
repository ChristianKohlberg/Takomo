//! Checklist — the verification surface: `/v1/projects/{project}/releases`,
//! `/v1/projects/{project}/checks`, `/v1/checks/{id}/cases`, `/v1/cases/{id}` and
//! the derived coverage / worklist / gate reports.
//!
//! Takomo stores; the agent computes. Nothing here generates a combinatorial
//! model or judges whether one is right — it validates shapes, enforces who may
//! record what, and persists. The single place that rule bends is a human
//! verdict, which requires a `human`-scoped token: "a person looked at this" is
//! the one claim an agent must not be able to make on a person's behalf.

use super::{
    body_object, first, get_i64, get_str, get_string_array, parse_i64_param, query_pairs,
    reject_unknown, require_str, ApiJson,
};
use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;
use crate::store::{
    CaseInput, CheckCreate, CheckFilter, CheckPatch, PolicyInput, ReleasePush, VerdictInput,
    MAX_CASES_PAGE, MAX_CHECKS_PAGE,
};
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::{json, Value};
use std::sync::Arc;

const RELEASE_FIELDS: [&str; 4] = ["ref", "note", "touched_paths", "orphan_globs"];
const CHECK_CREATE_FIELDS: [&str; 16] = [
    "epic",
    "initiative",
    "node",
    "environments",
    "title",
    "body",
    "precondition",
    "layer",
    "severity",
    "verification",
    "expiry_days",
    "expiry_releases",
    "cost_agent_minutes",
    "cost_human_minutes",
    "globs",
    "metadata",
];
const CHECK_PATCH_FIELDS: [&str; 16] = [
    "epic",
    "initiative",
    "node",
    "environments",
    "title",
    "body",
    "precondition",
    "layer",
    "severity",
    "verification",
    "expiry_days",
    "expiry_releases",
    "cost_agent_minutes",
    "cost_human_minutes",
    "globs",
    "metadata_merge",
];
const CASES_FIELDS: [&str; 2] = ["cases", "prune"];
const CASE_FIELDS: [&str; 4] = ["key", "label", "assignment", "seeded"];
const VERDICT_FIELDS: [&str; 5] = ["verdict", "note", "release", "actor_kind", "environment"];
const POLICY_FIELDS: [&str; 4] = ["epic", "verification", "expiry_days", "expiry_releases"];

/// Read a field that is present-but-null distinctly from absent. An override slot
/// needs all three states: absent (leave alone), null (clear and inherit again),
/// value (set). Collapsing null into absent would make an inherited policy
/// impossible to restore once overridden.
fn override_str(
    obj: &serde_json::Map<String, Value>,
    key: &str,
) -> ApiResult<Option<Option<String>>> {
    match obj.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => Ok(Some(Some(s.clone()))),
        Some(_) => Err(ApiError::bad_request(
            "validation.field_type",
            format!("Field '{key}' must be a string or null."),
        )),
    }
}

fn override_i64(obj: &serde_json::Map<String, Value>, key: &str) -> ApiResult<Option<Option<i64>>> {
    match obj.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::Number(n)) => n.as_i64().map(|v| Some(Some(v))).ok_or_else(|| {
            ApiError::bad_request(
                "validation.field_type",
                format!("Field '{key}' must be a whole number or null."),
            )
        }),
        Some(_) => Err(ApiError::bad_request(
            "validation.field_type",
            format!("Field '{key}' must be a whole number or null."),
        )),
    }
}

// ---------------------------------------------------------------------------
// Releases
// ---------------------------------------------------------------------------

/// POST /v1/projects/{project}/releases (write) — record a release.
///
/// The pusher supplies `touched_paths` and `orphan_globs` because it has the tree
/// checked out; Takomo clones nothing. That keeps releases something the merging
/// agent reports rather than an integration the server owns, and it is the
/// cheapest possible place to learn which globs now match no files.
pub async fn push_release(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    ctx.require_project(&project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &RELEASE_FIELDS)?;
    let req = ReleasePush {
        project: project.clone(),
        reference: require_str(obj, "ref")?,
        note: get_str(obj, "note")?,
        touched_paths: get_string_array(obj, "touched_paths")?.unwrap_or_default(),
        orphan_globs: get_string_array(obj, "orphan_globs")?.unwrap_or_default(),
    };
    let (release, impact) = state.store.push_release(&req, &ctx.actor)?;
    state.wake();
    let mut out = release.to_json();
    out["impact"] = impact.to_json();
    Ok((StatusCode::CREATED, Json(out)))
}

/// GET /v1/projects/{project}/releases?limit= (read) — newest first.
pub async fn list_releases(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    let pairs = query_pairs(raw.as_deref());
    let limit = super::parse_i64_param(&pairs, "limit")?
        .unwrap_or(50)
        .clamp(1, 500) as usize;
    let items = state.store.list_releases(&project, limit)?;
    Ok(Json(
        json!({ "items": items.iter().map(|r| r.to_json()).collect::<Vec<_>>() }),
    ))
}

// ---------------------------------------------------------------------------
// Checks
// ---------------------------------------------------------------------------

/// POST /v1/projects/{project}/checks (write) — declare a check.
/// The node must be a section of this project's plan.
///
/// Checked HERE rather than in the store, and that is the whole point: nodes
/// live in the CRDT document, not in a table. A store-side check queried
/// `mindmap_nodes`, which is the legacy table `adopt_legacy_nodes` emptied — so
/// it could never find a node created today and refused every real id. Only this
/// layer can open the room.
///
/// A project holds at most one plan, which is what makes a bare node id
/// resolvable at all.
async fn validate_check_node(state: &Arc<AppState>, project: &str, node: &str) -> ApiResult<()> {
    let (maps, _) = state
        .store
        .list_mindmaps(&crate::store::MindmapListFilter {
            project: Some(project.to_string()),
            allowed_projects: None,
            status: None,
            q: None,
            limit: 2,
            offset: 0,
        })?;
    for map in &maps {
        let room = crate::api::docsync::open_room(state, &map.id).await?;
        let found = room.read(|doc| {
            // The third element is the typed node list; the first two are the
            // JSON the read routes serve.
            let (_, _, nodes) = crate::store::mindmapdoc::snapshot(doc, &map.id);
            nodes.iter().any(|n| n.id == node)
        });
        if found {
            return Ok(());
        }
    }
    Err(ApiError::validation(
        "validation.check_node",
        format!(
            "No node '{node}' in this project's plan. A check names the part of the plan it \
             verifies, so the id has to be one."
        ),
    )
    .remedy(
        "Read the plan with takomo_plan_read or GET /v1/mindmaps/{id} — every section carries \
         its node id — or leave `node` out to file a check about no part in particular."
            .to_string(),
    ))
}

pub async fn create_check(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    ctx.require_project(&project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &CHECK_CREATE_FIELDS)?;
    if let Some(n) = get_str(obj, "node")? {
        validate_check_node(&state, &project, &n).await?;
    }
    let req = CheckCreate {
        project: project.clone(),
        epic: get_str(obj, "epic")?,
        initiative: get_str(obj, "initiative")?,
        node: get_str(obj, "node")?,
        environments: get_string_array(obj, "environments")?.unwrap_or_default(),
        title: require_str(obj, "title")?,
        body: get_str(obj, "body")?.unwrap_or_default(),
        precondition: get_str(obj, "precondition")?.unwrap_or_default(),
        layer: get_str(obj, "layer")?,
        severity: get_str(obj, "severity")?,
        verification: get_str(obj, "verification")?,
        expiry_days: get_i64(obj, "expiry_days")?,
        expiry_releases: get_i64(obj, "expiry_releases")?,
        cost_agent_minutes: get_i64(obj, "cost_agent_minutes")?,
        cost_human_minutes: get_i64(obj, "cost_human_minutes")?,
        globs: get_string_array(obj, "globs")?.unwrap_or_default(),
        metadata: obj.get("metadata").filter(|v| !v.is_null()).cloned(),
    };
    let check = state.store.create_check(&req, &ctx.actor)?;
    state.wake();
    Ok((StatusCode::CREATED, Json(check.to_json())))
}

/// GET /v1/projects/{project}/checks?epic=&severity=&layer=&archived= (read).
///
/// `epic=none` narrows to ungrouped checks, which is how you find work nobody
/// filed under an epic — the same gap the roadmap's `unparented` bucket exists
/// for.
pub async fn list_checks(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    let pairs = query_pairs(raw.as_deref());
    let epic = match first(&pairs, "epic") {
        Some("none") => Some(String::new()),
        other => other.map(str::to_string),
    };
    let initiative = match first(&pairs, "initiative") {
        Some("none") => Some(String::new()),
        other => other.map(str::to_string),
    };
    // `?node=none` asks the opposite question, and it is the useful one: which
    // checks are about no part of the plan in particular.
    let node = match first(&pairs, "node") {
        Some("none") => Some(String::new()),
        other => other.map(str::to_string),
    };
    let filter = CheckFilter {
        project: project.clone(),
        epic,
        initiative,
        node,
        severity: first(&pairs, "severity").map(str::to_string),
        layer: first(&pairs, "layer").map(str::to_string),
        include_archived: first(&pairs, "archived") == Some("include"),
        with_policy: true,
        limit: parse_i64_param(&pairs, "limit")?,
    };
    let limit = filter
        .limit
        .unwrap_or(MAX_CHECKS_PAGE)
        .clamp(1, MAX_CHECKS_PAGE);
    let (checks, total) = state.store.list_checks(&filter)?;
    Ok(Json(super::paged(
        checks.iter().map(|l| l.to_json()).collect::<Vec<_>>(),
        total,
        limit,
        "Raise the page size with ?limit=N (max 200), or narrow with ?epic=/?initiative=/?severity=/?layer=.",
    )))
}

/// GET /v1/checks/{id} (read) — the check plus its resolved policy and case counts.
pub async fn get_check(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let check = state.store.get_check(&id)?;
    ctx.require_project(&check.project)?;
    Ok(Json(check.to_json()))
}

/// PATCH /v1/checks/{id} (write). Send an override as null to clear it and inherit
/// again.
pub async fn patch_check(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let existing = state.store.get_check(&id)?;
    ctx.require_project(&existing.project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &CHECK_PATCH_FIELDS)?;
    let patch = CheckPatch {
        title: get_str(obj, "title")?,
        body: get_str(obj, "body")?,
        precondition: get_str(obj, "precondition")?,
        layer: get_str(obj, "layer")?,
        severity: get_str(obj, "severity")?,
        verification: override_str(obj, "verification")?,
        expiry_days: override_i64(obj, "expiry_days")?,
        expiry_releases: override_i64(obj, "expiry_releases")?,
        cost_agent_minutes: override_i64(obj, "cost_agent_minutes")?,
        cost_human_minutes: override_i64(obj, "cost_human_minutes")?,
        globs: get_string_array(obj, "globs")?,
        metadata_merge: obj.get("metadata_merge").cloned(),
        epic: override_str(obj, "epic")?,
        initiative: override_str(obj, "initiative")?,
        node: override_str(obj, "node")?,
        environments: get_string_array(obj, "environments")?,
    };
    let check = state.store.patch_check(&id, &patch, &ctx.actor)?;
    state.wake();
    Ok(Json(check.to_json()))
}

/// DELETE /v1/checks/{id} (write) — archive it. Cases and verdict history survive:
/// a check no longer worth running is still evidence of what was once verified.
pub async fn archive_check(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let existing = state.store.get_check(&id)?;
    ctx.require_project(&existing.project)?;
    let check = state.store.archive_check(&id, &ctx.actor)?;
    state.wake();
    Ok(Json(check.to_json()))
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// PUT /v1/checks/{id}/cases (write) — file the generated case set.
///
/// Upsert by `key`. A case still present keeps its verdicts; one that vanished is
/// retired, not deleted; one that returns is revived. That is what makes
/// regenerating a model after adding a parameter safe. `prune: false` extends the
/// set instead of replacing it.
pub async fn file_cases(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let check = state.store.get_check(&id)?;
    ctx.require_project(&check.project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &CASES_FIELDS)?;
    let prune = match obj.get("prune") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            return Err(ApiError::bad_request(
                "validation.field_type",
                "Field 'prune' must be a boolean.",
            ))
        }
    };
    let raw = obj.get("cases").ok_or_else(|| {
        ApiError::validation("validation.cases", "Field 'cases' is required.")
            .remedy("Send {\"cases\": [{\"key\": \"...\", \"assignment\": {...}}]}.".to_string())
    })?;
    let items = raw.as_array().ok_or_else(|| {
        ApiError::bad_request(
            "validation.field_type",
            "Field 'cases' must be an array of objects.",
        )
    })?;
    let mut cases = Vec::with_capacity(items.len());
    for item in items {
        let o = body_object(item)?;
        reject_unknown(o, &CASE_FIELDS)?;
        cases.push(CaseInput {
            key: require_str(o, "key")?,
            label: get_str(o, "label")?.unwrap_or_default(),
            assignment: o.get("assignment").cloned().unwrap_or(Value::Null),
            seeded: matches!(o.get("seeded"), Some(Value::Bool(true))),
        });
    }
    let outcome = state.store.file_cases(&id, &cases, prune, &ctx.actor)?;
    state.wake();
    Ok(Json(outcome.to_json()))
}

/// GET /v1/checks/{id}/cases?retired=include (read).
pub async fn list_cases(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let check = state.store.get_check(&id)?;
    ctx.require_project(&check.project)?;
    let pairs = query_pairs(raw.as_deref());
    let include_retired = first(&pairs, "retired") == Some("include");
    let limit_arg = parse_i64_param(&pairs, "limit")?;
    let offset = parse_i64_param(&pairs, "offset")?.unwrap_or(0).max(0);
    let limit = limit_arg.unwrap_or(MAX_CASES_PAGE).clamp(1, MAX_CASES_PAGE);
    let (cases, total) = state
        .store
        .list_cases(&id, include_retired, limit_arg, Some(offset))?;
    let mut out = super::paged(
        cases.iter().map(|c| c.to_json()).collect::<Vec<_>>(),
        total,
        limit,
        &format!(
            "Read the next page with ?offset={}&limit={limit} (max page 500), and repeat while \
             offset+limit is below total. Cases are ordered by key, which is stable, so the \
             pages do not shift under you.",
            offset + limit
        ),
    );
    out["check"] = json!(id);
    out["offset"] = json!(offset);
    Ok(Json(out))
}

/// GET /v1/cases/{id} (read) — the case plus its full verdict history.
pub async fn get_case(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let (case, history, people) = state.store.get_case(&id)?;
    let check = state.store.get_check(&case.check)?;
    ctx.require_project(&check.project)?;
    let mut out = case.to_json();
    out["history"] = json!(history.iter().map(|v| v.to_json()).collect::<Vec<_>>());
    // Every `user` id in this payload, resolved once and keyed by that id, rather
    // than the same person repeated inline on each verdict they gave. "Who
    // approved this?" is the question this read exists to answer, and it is
    // answered here rather than by sending the reader back to /v1/users.
    //
    // Empty when nothing here names anybody — a project whose verdicts predate the
    // directory, or an agent-only case.
    out["people"] = json!(people
        .iter()
        .map(|p| (p.id.clone(), p.to_ref_json()))
        .collect::<serde_json::Map<_, _>>());
    Ok(Json(out))
}

/// POST /v1/cases/{id}/verdict (write) — record a verdict.
///
/// `actor_kind` defaults to `agent`; sending `human` requires the `human` scope.
/// That gate is the point: an agent may record what it observed, but only a
/// person's token can assert that a person approved it — the same line
/// `ask-a-human` draws.
pub async fn record_verdict(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let (case, _, _) = state.store.get_case(&id)?;
    let check = state.store.get_check(&case.check)?;
    ctx.require_project(&check.project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &VERDICT_FIELDS)?;
    let verdict = require_str(obj, "verdict")?;
    let actor_kind = get_str(obj, "actor_kind")?.unwrap_or_else(|| "agent".to_string());
    if actor_kind == "human" {
        ctx.require_scope("human").map_err(|_| {
            ApiError::new(
                StatusCode::FORBIDDEN,
                "forbidden.human_scope",
                "Recording a human verdict needs a token with the 'human' scope.",
            )
            .remedy(
                "An agent can record what it observed with actor_kind 'agent'. Only a \
                 person's token may assert that a person approved a case."
                    .to_string(),
            )
        })?;
    }
    let note = get_str(obj, "note")?;
    let release = get_str(obj, "release")?;
    let environment = get_str(obj, "environment")?;
    let out = state.store.record_verdict(&VerdictInput {
        case: &id,
        actor_kind: &actor_kind,
        actor: &ctx.actor,
        // Who the server can say gave this verdict. Recorded, not checked: what a
        // credential may assert here is still its scopes.
        user: ctx.user.as_deref(),
        verdict: &verdict,
        note: note.as_deref(),
        release: release.as_deref(),
        environment: environment.as_deref(),
    })?;
    state.wake();
    Ok(Json(out.to_json()))
}

// ---------------------------------------------------------------------------
// Policy and reports
// ---------------------------------------------------------------------------

/// GET /v1/initiatives/{id}/verification (read) — this initiative's standing.
///
/// A sub-resource rather than a field on the initiative: the rollup costs a
/// checks-and-cases scan, and `Initiative::to_json` is shared by the list read,
/// so inlining it would make listing 200 initiatives pay that scan 200 times.
pub async fn initiative_verification(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let out = state.store.initiative_verification(&id)?;
    ctx.require_project(out["project"].as_str().unwrap_or_default())?;
    Ok(Json(out))
}

/// PUT /v1/projects/{project}/checklist/policy (write) — set the project default
/// or, with `epic`, an epic-level override. Send a field as null to clear it.
pub async fn put_policy(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    ctx.require_project(&project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &POLICY_FIELDS)?;
    let epic = get_str(obj, "epic")?;
    let input = PolicyInput {
        verification: override_str(obj, "verification")?,
        expiry_days: override_i64(obj, "expiry_days")?,
        expiry_releases: override_i64(obj, "expiry_releases")?,
    };
    let out = state
        .store
        .set_checklist_policy(&project, epic.as_deref(), &input, &ctx.actor)?;
    Ok(Json(out))
}

/// GET /v1/projects/{project}/checklist/policy (read).
pub async fn get_policies(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    let items = state.store.list_checklist_policies(&project)?;
    Ok(Json(json!({ "items": items })))
}

/// GET /v1/projects/{project}/checklist/coverage (read).
pub async fn coverage(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    Ok(Json(state.store.checklist_coverage(&project)?))
}

/// GET /v1/projects/{project}/checklist/worklist (read).
pub async fn worklist(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    Ok(Json(state.store.checklist_worklist(&project)?))
}

/// GET /v1/projects/{project}/checklist/gate (read).
pub async fn gate(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    Ok(Json(state.store.checklist_gate(&project)?))
}
