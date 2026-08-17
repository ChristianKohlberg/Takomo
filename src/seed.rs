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
use crate::schedule::Cadence;
use crate::store::{
    AskRequest, QuestionFilter, ScheduleCreate, ScheduleTemplate, Store, TicketCreate,
    TicketListFilter, TimeoutAction,
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
        None,
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

    // --- two schedules, so /schedules is worth opening ---------------------
    //
    // The page's whole point is the lineage strip, and a strip with no history is
    // eight empty boxes. Both are anchored to a FIXED instant in the past rather
    // than "now minus N days", so the fixture is deterministic: the same slots
    // every time the template is baked, whatever day it is baked on.
    //
    // Nothing is materialized here. The sweeper does that on its own tick, which
    // keeps the seed honest — what you see on the page is the real firing path,
    // not tickets the seeder drew to look like one.
    let anchor = 1_767_225_600_000; // 2026-01-01T00:00:00Z
    let weekly = store.create_schedule(
        &ScheduleCreate {
            project: PROJECT.to_string(),
            name: "Weekly review".to_string(),
            cadence: Cadence {
                every: "week".to_string(),
                interval: 1,
                on: vec!["mon".to_string()],
                day: None,
                at: "09:00".to_string(),
                tz: Some("Europe/Berlin".to_string()),
            },
            template: ScheduleTemplate {
                title: "Weekly review — {week}".to_string(),
                body: "Skim the board: what moved, what stalled, what needs a decision.\n\n                       Close this out with a comment naming anything you re-prioritised."
                    .to_string(),
                ty: None,
                priority: None,
                labels: vec!["ritual".to_string()],
                tags: vec![],
            },
            starts_at: Some(anchor),
            ends_at: None,
            rationale: None,
        },
        SEEDER,
        false,
    )?;
    // …and one an agent proposed, left pending, so the confirm row and the nav
    // badge both have something real to show.
    store.create_schedule(
        &ScheduleCreate {
            project: PROJECT.to_string(),
            name: "Rotate the deploy key".to_string(),
            cadence: Cadence {
                every: "month".to_string(),
                interval: 1,
                on: vec![],
                day: Some(1),
                at: "09:00".to_string(),
                tz: Some("Europe/Berlin".to_string()),
            },
            template: ScheduleTemplate {
                title: "Rotate the deploy key — {month}".to_string(),
                body: "Rotate, then attach the commit that records the new fingerprint."
                    .to_string(),
                ty: None,
                priority: Some("high".to_string()),
                labels: vec!["ops".to_string()],
                tags: vec![],
            },
            starts_at: Some(anchor),
            ends_at: None,
            rationale: Some(
                "Rotated by hand three months running. Same work each time, so proposing a cadence."
                    .to_string(),
            ),
        },
        agent,
        true,
    )?;
    // Give the weekly review a history, through the real firing path: rewind to a
    // past slot, fire, repeat. Nothing is hand-inserted, so what the strip shows
    // is what materialization actually produces.
    //
    // Deterministic in SHAPE rather than in dates — the anchor is pinned, so every
    // slot is a real grid point, but how many lie between it and today depends on
    // when the template is baked.
    let weekly_cadence = Cadence {
        every: "week".to_string(),
        interval: 1,
        on: vec!["mon".to_string()],
        day: None,
        at: "09:00".to_string(),
        tz: Some("Europe/Berlin".to_string()),
    };
    let anchor_dt = chrono::DateTime::from_timestamp_millis(anchor).unwrap_or_default();
    let mut past: Vec<i64> = Vec::new();
    let mut cursor = anchor - 1;
    let now = now_ms();
    while past.len() < 400 {
        match weekly_cadence.next_slot_after(
            chrono::DateTime::from_timestamp_millis(cursor).unwrap_or_default(),
            anchor_dt,
        ) {
            Some(next) => {
                let ms = next.timestamp_millis();
                if ms > now {
                    break;
                }
                past.push(ms);
                cursor = ms;
            }
            None => break,
        }
    }
    // The last seven slots that have already come round: enough to fill the
    // strip, which shows eight.
    let recent: Vec<i64> = past.iter().rev().take(7).rev().copied().collect();
    let mut fired: Vec<String> = Vec::new();
    if let Some(first) = recent.first() {
        store.rewind_next_slot(&weekly.id, *first)?;
        for _ in &recent {
            if let Some(ticket) = store.run_schedule_now(&weekly.id, SEEDER)? {
                fired.push(ticket);
            }
        }
    }
    // Take four of them all the way to `done`, so the strip reads as a real
    // cadence someone has been keeping unevenly — done, done, missed, done —
    // rather than a wall of one colour.
    for (i, ticket) in fired.iter().enumerate() {
        if i % 2 == 1 {
            continue; // leave the odd ones unfinished: `not fulfilled`
        }
        advance(store, ticket, "spec", SEEDER, None)?;
        advance(store, ticket, "ready", SEEDER, None)?;
        let fence = claim(store, ticket, agent)?;
        advance(store, ticket, "implementing", agent, Some(fence))?;
        advance(store, ticket, "review", agent, Some(fence))?;
        advance(store, ticket, "done", SEEDER, None)?;
    }

    initiative(store)?;

    // Counted, not hardcoded, so the summary can't drift from the content.
    Ok(SeedSummary {
        project: PROJECT.to_string(),
        tickets: seeded_tickets(store)?.len(),
        questions: seeded_questions(store)?.len(),
        skipped: false,
    })
}

/// One day, in milliseconds — the unit every `origin_at` below is offset by.
const DAY_MS: i64 = 86_400_000;

/// Seed the demo initiative as a DOCUMENT, not just a pile of entries.
///
/// Three `view` entries (one per pane) carry the prose; `[n]` marks inside it
/// index that pane's own `meta.cites` list, so an author only ever needs local
/// numbering. `thread` entries are margin notes anchored to a paragraph. Every
/// other entry here is evidence: citable, and listed in the lineage footer.
///
/// None of this needed a schema change — `kind` has always been a free-form slug
/// and `meta` a free-form JSON object on every entry. The document is reduced
/// from these rows on read and never stored, exactly like `rollup`.
///
/// Evidence is appended FIRST because a view cites entries by id, and the ids do
/// not exist until the rows do.
fn initiative(store: &Store) -> ApiResult<()> {
    let now = now_ms();
    let ini = store.create_initiative(
        PROJECT,
        &crate::store::InitiativeCreate {
            title: "Split billing on shared invoices".to_string(),
            summary: Some(
                "Multi-entity customers want one invoice divided before it reaches their AP system."
                    .to_string(),
            ),
            labels: vec!["billing".to_string()],
            tags: vec!["person:ada".to_string(), "domain:billing".to_string()],
            ..Default::default()
        },
        SEEDER,
    )?;

    // `origin_at` is when the content was WRITTEN; `created_at` is when it
    // landed here. They are set apart on purpose — the five-month gap on the
    // transcript is the most honest thing in this fixture.
    // `origin` marks an entry as HOW THE IDEA ARRIVED — quoted at the top of the
    // document, above every pane, because each pane is somebody's interpretation
    // and this is the input they are accountable to.
    let evidence =
        |kind: &str, title: &str, text: &str, source: &str, age_days: i64, origin: bool| {
            store
                .append_initiative_entry(
                    &ini.id,
                    &crate::store::EntryCreate {
                        kind: kind.to_string(),
                        title: Some(title.to_string()),
                        text: text.to_string(),
                        source: source.to_string(),
                        origin_at: Some(now - age_days * DAY_MS),
                        meta: origin.then(|| json!({ "origin": true })),
                        ..Default::default()
                    },
                    SEEDER,
                )
                .map(|(entry, _)| entry.id)
        };

    let call = evidence(
        "transcript",
        "QBR call, Nordwind",
        "\"We get one invoice for six sites. Someone here retypes it into six lines every month. That person is me.\"",
        "person:kunde-nordwind",
        152,
        true,
    )?;
    let invoice = evidence(
        "sample-data",
        "Nordwind, March invoice",
        "Page 3 divides a shared licence line pro-rata across all six sites — one line becomes six.",
        "agent:w3",
        150,
        false,
    )?;
    let talberg = evidence(
        "note",
        "Talberg says the same thing",
        "Raised unprompted during onboarding, six weeks after Nordwind. Four legal entities, same manual workaround.",
        "person:ada",
        44,
        true,
    )?;
    let code = evidence(
        "code-research",
        "src/store/billing.rs:142-190",
        "One ledger row, one account. The PDF renders from the ledger at request time, so nothing sits in between to divide.",
        "agent:w1",
        5,
        false,
    )?;
    let scan = evidence(
        "research",
        "How three competitors do it",
        "Stripe has no native split. Chargebee splits by subscription. All three divide shared lines, and all three call it \"allocation\".",
        "agent:w3",
        1,
        false,
    )?;

    let view = |pane: &str, text: &str, cites: Vec<&str>| {
        store.append_initiative_entry(
            &ini.id,
            &crate::store::EntryCreate {
                kind: "view".to_string(),
                text: text.to_string(),
                source: "agent:w1".to_string(),
                meta: Some(json!({ "pane": pane, "cites": cites })),
                ..Default::default()
            },
            SEEDER,
        )
    };

    view(
        "business",
        "Two multi-entity customers re-key a single invoice into their AP system by hand, every month[1]. \
         Nordwind bills six sites; Talberg four legal entities[3]. Neither asked for a feature — both described \
         the same manual workaround, unprompted, six weeks apart.\n\n\
         Whether this is a segment or a coincidence is unknown. Nobody has counted how many accounts bill \
         more than one site.\n\n\
         The customer defines cost centres and receives one invoice per centre. Nordwind's invoice also divides \
         a shared licence line across all six sites[2], which is where the two customers stop looking alike — \
         and the open question is whether we promise whole-amount splits or allocation.",
        vec![&call, &invoice, &talberg],
    )?;

    view(
        "technical",
        "An invoice is one ledger row carrying one account, and the PDF renders from it at request time[1]. \
         There is no line-item table. This is a data-model change, not a rendering change.\n\n\
         A new invoice_splits child table keyed on the ledger row. Cost centres reuse the existing project tag \
         registry, so there is no second identity concept to keep consistent.\n\n\
         Allocating a shared line across centres would need a second table and a rounding policy, so that the \
         parts sum exactly to the whole.",
        vec![&code],
    )?;

    view(
        "verification",
        "Three things no agent can answer. Does Nordwind's AP system import one file per vendor or one per \
         site? Is the shared licence line divided by headcount or equally[1]? Is a per-entity invoice a \
         separate tax point[2]?\n\n\
         What must be true before this ships: split amounts sum exactly to the parent total, remainder cent \
         included; a split cannot be issued against a closed ledger row; Nordwind's March invoice reproduces \
         from the real ledger row[1]; invoices with no split rows render byte-identically to today.\n\n\
         Each names the layer it holds at, because a rule enforced only in the UI is not the same claim as one \
         enforced in the API. On distil these become a Checklist check rather than a blank page.",
        vec![&invoice, &scan],
    )?;

    let thread = |pane: &str, para: i64, state: &str, text: &str, source: &str| {
        store.append_initiative_entry(
            &ini.id,
            &crate::store::EntryCreate {
                kind: "thread".to_string(),
                text: text.to_string(),
                source: source.to_string(),
                meta: Some(json!({ "pane": pane, "para": para, "state": state })),
                ..Default::default()
            },
            SEEDER,
        )
    };

    thread(
        "business",
        1,
        "running",
        "I am not building for two customers. How many accounts bill more than one site?",
        "person:ada",
    )?;
    thread(
        "business",
        2,
        "open",
        "All three competitors call this \"allocation\", not \"split\". The word customers arrive with is not the one we use.",
        "agent:w3",
    )?;
    thread(
        "technical",
        2,
        "open",
        "Who owns the remainder cent? Every allocation scheme gets this wrong once and then never again.",
        "person:ada",
    )?;

    // A proposed amendment, still undecided: the competitor scan found that the
    // whole category says "allocation" where we say "split", which changes the
    // Business view's third paragraph rather than adding a finding beside it.
    //
    // `proposed` keeps it OUT of the live pane. It is offered as a diff, and a
    // person accepts or rejects it — at which point the accepted prose is
    // appended as a real `view` and the decision recorded. Nothing is edited.
    store.append_initiative_entry(
        &ini.id,
        &crate::store::EntryCreate {
            kind: "view".to_string(),
            title: Some("Adopt the word the rest of the category uses".to_string()),
            text: "Two multi-entity customers re-key a single invoice into their AP system by hand, every month[1]. \
                   Nordwind bills six sites; Talberg four legal entities[3]. Neither asked for a feature — both described \
                   the same manual workaround, unprompted, six weeks apart.\n\n\
                   Whether this is a segment or a coincidence is unknown. Nobody has counted how many accounts bill \
                   more than one site.\n\n\
                   The customer defines cost centres and receives one invoice per centre. Shared lines are allocated \
                   across those centres by a customer-chosen rule — headcount, revenue or equal shares[4]. We use the \
                   word the rest of the category uses, because it is the word customers arrive with[2]."
                .to_string(),
            source: "agent:w3".to_string(),
            meta: Some(json!({
                "pane": "business",
                "cites": [&call, &scan, &talberg, &invoice],
                "proposed": true,
            })),
            ..Default::default()
        },
        SEEDER,
    )?;

    Ok(())
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
    store.list_questions(&filter).map(|(items, _total)| items)
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
