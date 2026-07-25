//! Demo content for a local instance — what `backlot up` puts in front of you.
//!
//! This lives in the library rather than the binary because it is not a thin
//! wrapper over one store call: it drives the **real state machine**. Reaching
//! `implementing` means claiming and presenting a fence; reaching `ready` means
//! satisfying a `scope:human` gate; parking a ticket in `needs-decision` means a
//! blocking question finding a self-service edge into a blocked state. So the
//! seed doubles as a traversability check on the factory-default workflow: if an
//! edge gains a `requires`, or a state is renamed, seeding stops reaching half
//! the board — and the test below says so.

use crate::error::ApiResult;
use crate::ids::now_ms;
use crate::store::{
    AskRequest, QuestionFilter, Store, TicketCreate, TicketListFilter, TimeoutAction,
};
use serde_json::json;
use std::collections::HashSet;

/// The demo project's id; it becomes the seeded tickets' id prefix.
pub const PROJECT: &str = "demo";
/// Attribution for everything the seeder does itself.
const SEEDER: &str = "human:seed";

/// What a seed run did, for the CLI to report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedSummary {
    pub project: String,
    pub tickets: usize,
    pub questions: usize,
    /// True when the project already existed and nothing was written.
    pub skipped: bool,
}

/// The scopes the seeder asserts for its own transitions. Seeding talks to the
/// database directly, and shell access to the database is the root of trust
/// (spec/auth.md), so it simply holds the `human` scope the workflow gates want.
fn seed_scopes() -> HashSet<String> {
    ["read", "write", "human"]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

/// A `TicketCreate` for the demo project. Tickets are always born in the
/// workflow's initial state; callers move them with `advance`.
fn ticket(ty: &str, title: &str, body: &str, priority: &str, labels: &[&str]) -> TicketCreate {
    TicketCreate {
        project: PROJECT.to_string(),
        ty: Some(ty.to_string()),
        title: title.to_string(),
        body: Some(body.to_string()),
        priority: Some(priority.to_string()),
        labels: labels.iter().map(|l| (*l).to_string()).collect(),
        ..Default::default()
    }
}

fn add(store: &Store, req: TicketCreate) -> ApiResult<String> {
    let (t, _, _) = store.create_ticket(&req, SEEDER, None)?;
    Ok(t.id)
}

fn advance(store: &Store, id: &str, to: &str, actor: &str, fence: Option<i64>) -> ApiResult<()> {
    store.transition(id, to, Some("seeded"), fence, actor, &seed_scopes())?;
    Ok(())
}

/// Claim a ticket for `actor` and return the lease fence, so the follow-up
/// transition can present it (`ready -> implementing` requires the claim).
fn claim(store: &Store, id: &str, actor: &str) -> ApiResult<i64> {
    let (_, lease) = store.claim_ticket(id, actor, None)?;
    Ok(lease.fence)
}

/// Raise a question and return its id, so a caller can add a follow-up turn.
fn ask(store: &Store, req: AskRequest, actor: &str) -> ApiResult<String> {
    let (q, _) = store.ask_question(&req, actor)?;
    Ok(q.id)
}

/// Seed the `dev` preset: a demo project with tickets in every workflow state
/// and questions of every kind. Idempotent — a second call is a no-op.
pub fn dev(store: &Store) -> ApiResult<SeedSummary> {
    // Idempotent by project existence. The sqlite template is baked once per
    // seed-content hash and restored by copy, but a hand-run `takomo seed`
    // against a live database would otherwise stack a second demo project.
    if store.get_project(PROJECT)?.is_some() {
        return Ok(SeedSummary {
            project: PROJECT.to_string(),
            tickets: 0,
            questions: 0,
            skipped: true,
        });
    }
    store.create_project(PROJECT, "Demo — agent fleet", None, SEEDER)?;

    let agent = "agent:runner-1";
    let agent2 = "agent:runner-2";
    let day = 86_400_000;

    // --- an epic in `spec`, carrying the advisory (epic-level) question ------
    let epic = add(
        store,
        ticket(
            "epic",
            "Billing revamp",
            "Umbrella for the billing rework: new ledger, migration off billing_v1, invoice PDFs.",
            "high",
            &["billing", "epic"],
        ),
    )?;
    advance(store, &epic, "spec", SEEDER, None)?;
    ask(
        store,
        AskRequest {
            ticket: epic.clone(),
            mode: Some("advisory".to_string()),
            kind: "choose".to_string(),
            title: "Rewrite or incremental for the billing epic?".to_string(),
            body: "A rewrite lands a clean ledger but freezes billing changes for ~6 weeks. \
                   Incremental keeps shipping but carries the v1 schema for longer."
                .to_string(),
            options: vec!["rewrite".to_string(), "incremental".to_string()],
            option_notes: vec![
                "Clean ledger, ~6 week freeze.".to_string(),
                "Ships continuously, v1 schema lingers.".to_string(),
            ],
            recommended: json!("incremental"),
            recommended_note: Some("Freezing billing over quarter-end is the bigger risk.".into()),
            confidence: Some(3),
            summary: Some("Direction for the whole billing epic.".to_string()),
            expertise: vec!["domain:product".to_string()],
            urgency: Some("normal".to_string()),
            ..Default::default()
        },
        agent,
    )?;

    // --- `needs-decision` via a blocking confirm, with a timeout fallback ----
    let migrate = add(
        store,
        TicketCreate {
            parent: Some(epic.clone()),
            ..ticket(
                "task",
                "Migrate off the billing_v1 table",
                "No reads in 90d. Copy forward, then drop.",
                "critical",
                &["billing", "migration"],
            )
        },
    )?;
    advance(store, &migrate, "spec", SEEDER, None)?;
    ask(
        store,
        AskRequest {
            ticket: migrate.clone(),
            kind: "confirm".to_string(),
            title: "OK to drop table billing_v1?".to_string(),
            body: "No reads in 90 days. I have the copy-forward ready and verified on a restore."
                .to_string(),
            recommended: json!(true),
            recommended_note: Some(
                "Backfill verified against a restore; rollback is a rename.".into(),
            ),
            confidence: Some(4),
            expertise: vec!["domain:billing".to_string()],
            urgency: Some("high".to_string()),
            expires_at: Some(now_ms() + day),
            on_timeout: Some(TimeoutAction::Recommended),
            ..Default::default()
        },
        agent,
    )?;

    // --- `needs-decision` via a blocking approve (expert-gated) -------------
    let pricing = add(
        store,
        ticket(
            "task",
            "Apply the 2026 price list to active subscriptions",
            "Re-prices ~1,800 live subscriptions. Not reversible without a credit run.",
            "high",
            &["billing", "pricing"],
        ),
    )?;
    advance(store, &pricing, "spec", SEEDER, None)?;
    ask(
        store,
        AskRequest {
            ticket: pricing.clone(),
            kind: "approve".to_string(),
            title: "Approve re-pricing 1,800 live subscriptions?".to_string(),
            body: "Dry run attached in the ticket body. Needs a billing owner, not just any human."
                .to_string(),
            expertise: vec!["domain:billing".to_string()],
            urgency: Some("critical".to_string()),
            summary: Some("Irreversible re-price of live subscriptions.".to_string()),
            ..Default::default()
        },
        agent2,
    )?;

    // --- `needs-decision` via a blocking clarify ----------------------------
    let webhook = add(
        store,
        ticket(
            "bug",
            "Webhook retries double-charge on 5xx",
            "Retry path re-runs the capture instead of resuming it.",
            "critical",
            &["billing", "bug"],
        ),
    )?;
    advance(store, &webhook, "spec", SEEDER, None)?;
    let clarify = ask(
        store,
        AskRequest {
            ticket: webhook.clone(),
            kind: "clarify".to_string(),
            title: "Which idempotency key should the capture retry reuse?".to_string(),
            body:
                "The provider accepts one key per capture attempt. Reuse the original, or derive \
                   a per-attempt key and reconcile after?"
                    .to_string(),
            expertise: vec!["domain:billing".to_string()],
            urgency: Some("critical".to_string()),
            ..Default::default()
        },
        agent,
    )?;
    // One question bounced back to the asking agent, so the inbox has something
    // in the `awaiting: agent` state ("researching") and not just a flat queue.
    store.request_followup(
        &clarify,
        SEEDER,
        "Before I decide: does the provider treat a re-used key as idempotent \
         across attempts, or only within one? Check their docs and report back.",
    )?;

    // --- `ready` -------------------------------------------------------------
    let ratelimit = add(
        store,
        ticket(
            "task",
            "Rate-limit POST /v1/questions per token",
            "A looping agent can flood the inbox; the write budget should cover asks too.",
            "normal",
            &["api", "hardening"],
        ),
    )?;
    advance(store, &ratelimit, "spec", SEEDER, None)?;
    advance(store, &ratelimit, "ready", SEEDER, None)?;

    // --- `implementing`, claimed (so the board shows a claim holder) ---------
    let palette = add(
        store,
        ticket(
            "task",
            "Port the Aquarelle palette to /inbox",
            "Second design increment: carry the board's token set into the triage surface.",
            "high",
            &["ui", "design"],
        ),
    )?;
    advance(store, &palette, "spec", SEEDER, None)?;
    advance(store, &palette, "ready", SEEDER, None)?;
    let fence = claim(store, &palette, agent)?;
    advance(store, &palette, "implementing", agent, Some(fence))?;
    store.add_comment(
        &palette,
        agent,
        "Folder rail and list pane done; reading pane next.",
    )?;

    // --- `review`, claimed, and blocked by the rate-limit work --------------
    let sweep = add(
        store,
        ticket(
            "bug",
            "Answer-link expiry sweep flakes under load",
            "The sweep and the lease sweeper contend on the same write lock.",
            "critical",
            &["questions", "bug"],
        ),
    )?;
    advance(store, &sweep, "spec", SEEDER, None)?;
    advance(store, &sweep, "ready", SEEDER, None)?;
    let fence = claim(store, &sweep, agent2)?;
    advance(store, &sweep, "implementing", agent2, Some(fence))?;
    advance(store, &sweep, "review", agent2, Some(fence))?;
    // Added by the lease holder, presenting its fence: while a ticket is
    // claimed, only the holder may touch its dependencies.
    store.add_dep(&sweep, &ratelimit, agent2, Some(fence))?;

    // --- `brief` -------------------------------------------------------------
    add(
        store,
        ticket(
            "spike",
            "Spike: Slack socket-mode transport",
            "Outbound WebSocket means no inbound port — viable for a tailnet deploy.",
            "low",
            &["integrations", "spike"],
        ),
    )?;

    // --- `done` --------------------------------------------------------------
    let favicon = add(
        store,
        ticket(
            "task",
            "Serve the octopus as favicon on both surfaces",
            "One SVG, served from the binary at /favicon.svg and /favicon.ico.",
            "low",
            &["ui"],
        ),
    )?;
    advance(store, &favicon, "spec", SEEDER, None)?;
    advance(store, &favicon, "ready", SEEDER, None)?;
    let fence = claim(store, &favicon, agent)?;
    advance(store, &favicon, "implementing", agent, Some(fence))?;
    advance(store, &favicon, "review", agent, Some(fence))?;
    advance(store, &favicon, "done", SEEDER, None)?;

    // --- `cancelled` ---------------------------------------------------------
    let redirect = add(
        store,
        ticket(
            "task",
            "Drop the legacy /v1/board redirect",
            "Superseded: /board is served directly.",
            "low",
            &["cleanup"],
        ),
    )?;
    advance(store, &redirect, "cancelled", SEEDER, None)?;

    // Counted, not hardcoded, so the summary can't drift from the content.
    Ok(SeedSummary {
        project: PROJECT.to_string(),
        tickets: seeded_tickets(store)?.len(),
        questions: seeded_questions(store)?.len(),
        skipped: false,
    })
}

/// Every ticket in the demo project.
fn seeded_tickets(store: &Store) -> ApiResult<Vec<crate::store::Ticket>> {
    let filter = TicketListFilter {
        project: Some(PROJECT.to_string()),
        ..Default::default()
    };
    let (tickets, _) = store.list_tickets(&filter, None, 500)?;
    Ok(tickets)
}

/// Every open question in the demo project.
fn seeded_questions(store: &Store) -> ApiResult<Vec<crate::store::Question>> {
    let filter = QuestionFilter {
        project: Some(PROJECT.to_string()),
        ..Default::default()
    };
    store.list_questions(&filter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::factory_default;
    use std::collections::HashSet;

    fn seeded() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = Store::open(tmp.path().join("seed.db")).expect("open store");
        dev(&store).expect("seed dev");
        (tmp, store)
    }

    /// The point of the seed: every state in the factory-default workflow is
    /// actually reachable by walking the machine. If an edge gains a `requires`
    /// the seeder cannot satisfy, or a state is renamed, this fails — which is
    /// the regression nothing else in the suite catches.
    #[test]
    fn seeds_every_workflow_state() {
        let (_tmp, store) = seeded();
        let tickets = seeded_tickets(&store).unwrap();
        let reached: HashSet<String> = tickets.iter().map(|t| t.state.clone()).collect();
        let declared: HashSet<String> = factory_default()
            .states
            .iter()
            .map(|s| s.id.clone())
            .collect();
        assert_eq!(
            declared, reached,
            "seed must populate every factory-default state exactly (declared vs reached)"
        );
    }

    /// The inbox is only worth looking at if it carries all four answer shapes,
    /// both modes, and one question already bounced back to its agent.
    #[test]
    fn seeds_every_question_kind_and_both_modes() {
        let (_tmp, store) = seeded();
        let questions = seeded_questions(&store).unwrap();

        let kinds: HashSet<&str> = questions.iter().map(|q| q.kind.as_str()).collect();
        assert_eq!(
            kinds,
            crate::store::QUESTION_KINDS.iter().copied().collect(),
            "every question kind should be represented"
        );

        let modes: HashSet<&str> = questions.iter().map(|q| q.mode.as_str()).collect();
        assert!(modes.contains("blocking") && modes.contains("advisory"));

        // The follow-up thread: one question is waiting on the agent, not a human.
        assert_eq!(
            questions.iter().filter(|q| q.awaiting == "agent").count(),
            1,
            "exactly one question should be bounced back to its asking agent"
        );
        // `approve` is the expert-gated kind, so it must name its expertise.
        let approve = questions.iter().find(|q| q.kind == "approve").unwrap();
        assert!(
            !approve.expertise.is_empty(),
            "an approve question must carry an expertise tag to gate on"
        );
    }

    /// Claims and dependencies are what make the board look live rather than flat.
    #[test]
    fn seeds_claims_and_a_dependency() {
        let (_tmp, store) = seeded();
        let tickets = seeded_tickets(&store).unwrap();
        assert!(
            tickets.iter().filter(|t| t.claim_holder.is_some()).count() >= 2,
            "at least two tickets should show a claim holder"
        );
        assert!(
            tickets.iter().any(|t| !t.blocked_by.is_empty()),
            "at least one ticket should be blocked by another"
        );
        assert!(
            tickets.iter().any(|t| t.parent.is_some()),
            "the epic should have at least one child"
        );
    }

    #[test]
    fn is_idempotent() {
        let (_tmp, store) = seeded();
        let before = seeded_tickets(&store).unwrap().len();

        let again = dev(&store).expect("second seed");
        assert!(again.skipped, "a second run must not write anything");
        assert_eq!(
            seeded_tickets(&store).unwrap().len(),
            before,
            "a second run must not duplicate tickets"
        );
    }

    #[test]
    fn summary_counts_match_the_store() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = Store::open(tmp.path().join("seed.db")).expect("open store");
        let summary = dev(&store).expect("seed dev");
        assert!(!summary.skipped);
        assert_eq!(summary.tickets, seeded_tickets(&store).unwrap().len());
        assert_eq!(summary.questions, seeded_questions(&store).unwrap().len());
    }
}
