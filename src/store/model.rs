//! Row models and wire (JSON) shapes.

use super::shares::ShareKind;
use crate::ids::iso;
use serde_json::{json, Value};

pub const TICKET_TYPES: [&str; 4] = ["epic", "task", "bug", "spike"];
pub const PRIORITIES: [&str; 4] = ["critical", "high", "normal", "low"];

pub const MAX_TITLE: usize = 300;
pub const MAX_BODY: usize = 131_072;
pub const MAX_COMMENT: usize = 65_536;
pub const MAX_METADATA: usize = 65_536;
/// Per-key cap on `links` keys (same bound as labels).
pub const MAX_LINK_KEY: usize = 100;
/// Per-value cap on `links` values — long enough for a URL, not a document.
pub const MAX_LINK_VALUE: usize = 8_192;
/// How many link keys a ticket may carry at once.
pub const MAX_LINKS: usize = 32;
/// Serialized `links` object size cap (same shape as `metadata`).
pub const MAX_LINKS_SIZE: usize = MAX_METADATA;

#[derive(Debug, Clone)]
pub struct Ticket {
    pub id: String,
    pub project: String,
    pub ty: String,
    pub parent: Option<String>,
    pub title: String,
    pub body: String,
    pub state: String,
    pub state_category: String,
    pub priority: String,
    pub labels: Vec<String>,
    /// Tag references onto this ticket, each a canonical `kind:handle` string
    /// (e.g. `person:ada`, `component:billing`). Reference metadata only — never
    /// affects claims, leases, or question routing.
    pub tags: Vec<String>,
    pub metadata: Value,
    pub links: Value,
    pub blocked_by: Vec<String>,
    pub claim_holder: Option<String>,
    /// When the lease expires — or None while `claim_holder` is set, for an
    /// epic claim taken without a TTL: held until released, never expiring.
    pub claim_expires_at: Option<i64>,
    /// When the current lease was granted; cleared with the claim. None on a
    /// claim granted before the column existed.
    pub claim_since: Option<i64>,
    /// The actor whose lease here ended by expiry, when nothing has claimed,
    /// released or revoked one since. Read [`Ticket::lapsed_holder`] rather than
    /// this field: between the expiry and the sweep the same fact is still
    /// recorded as an expired `claim_holder`, and that method covers both.
    pub lapsed_claim_holder: Option<String>,
    pub fence_seq: i64,
    pub version: i64,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// ISO timestamp when the ticket was archived, or None when active.
    /// Archived tickets are hidden from default list/ready/board/metrics views.
    pub archived_at: Option<String>,
    /// The schedule that materialized this ticket, or None for a hand-made one.
    ///
    /// Provenance and a link back, not a relationship: two tickets from the same
    /// schedule have no edge between them, and this column carries no foreign key
    /// so deleting the rule leaves the work — and the record of where it came
    /// from — intact.
    pub schedule: Option<String>,
    /// The calendar slot this ticket stands for. Unique per schedule, which is
    /// what makes exactly-one-ticket-per-slot structural.
    pub occurrence: Option<i64>,
    /// When this ticket stops counting as live work — the moment its schedule's
    /// next occurrence comes due, stamped at creation from the cadence alone.
    ///
    /// Expiry changes no state: an expired ticket is not archived, cancelled or
    /// transitioned. It leaves the ready queue and reads as `not_fulfilled`.
    pub expires_at: Option<i64>,
}

impl Ticket {
    /// Whether the claim is active at `now` (expired leases read as unclaimed).
    /// A `None` expiry on an active claim means it never expires — an epic
    /// claim taken without a TTL, held until released.
    pub fn active_claim(&self, now: i64) -> Option<(&str, Option<i64>)> {
        match (&self.claim_holder, self.claim_expires_at) {
            (Some(h), Some(exp)) if exp > now => Some((h.as_str(), Some(exp))),
            (Some(h), None) => Some((h.as_str(), None)),
            _ => None,
        }
    }

    /// Who the ticket's last lease belonged to, if it ended by **expiry** and
    /// nothing has claimed, released or revoked one since. `None` means there is
    /// nothing to resume: the ticket was never claimed, is claimed right now, or
    /// its last lease ended some other way.
    ///
    /// Two sources for one fact, because expiry is noticed twice: until the sweep
    /// (or the next write on the ticket) clears it, the lapsed lease is still
    /// recorded in `claim_holder`/`claim_expires_at`; afterwards it lives in
    /// `lapsed_claim_holder`. A caller that read only one of them would get a
    /// different answer depending on whether a 250ms timer had fired.
    pub fn lapsed_holder(&self, now: i64) -> Option<&str> {
        match (&self.claim_holder, self.claim_expires_at) {
            (Some(h), Some(exp)) if exp <= now => Some(h.as_str()),
            (Some(_), _) => None, // an active claim: nothing has lapsed
            (None, _) => self.lapsed_claim_holder.as_deref(),
        }
    }

    pub fn to_json(&self, now: i64) -> Value {
        let claim = self
            .active_claim(now)
            .map(|(h, exp)| {
                // `expires_at: null` = a claim with no expiry (an epic claim
                // held until released), not a missing field. `since` is when it
                // was granted — null only on rows that predate the column.
                json!({
                    "holder": h,
                    "expires_at": exp.map(iso),
                    "since": self.claim_since.map(iso),
                })
            })
            .unwrap_or(Value::Null);
        json!({
            "id": self.id,
            "project": self.project,
            "type": self.ty,
            "parent": self.parent,
            "title": self.title,
            "body": self.body,
            "state": self.state,
            "state_category": self.state_category,
            "priority": self.priority,
            "labels": self.labels,
            "tags": self.tags,
            "metadata": self.metadata,
            "links": self.links,
            "blocked_by": self.blocked_by,
            "claim": claim,
            "version": self.version,
            "created_by": self.created_by,
            "created_at": iso(self.created_at),
            "updated_at": iso(self.updated_at),
            "archived_at": self.archived_at,
            "schedule": self.schedule,
            "occurrence": self.occurrence.map(iso),
            "expires_at": self.expires_at.map(iso),
        })
    }

    /// Whether this ticket has stopped counting as live work.
    ///
    /// Only ever true for a scheduled ticket: a hand-made one has no
    /// `expires_at` and never expires. Terminal state wins — a ticket somebody
    /// finished is `done`, however late.
    pub fn is_expired(&self, now: i64) -> bool {
        matches!(self.expires_at, Some(exp) if exp <= now)
    }
}

#[derive(Debug, Clone)]
pub struct Comment {
    pub id: String,
    pub ticket: String,
    pub author: String,
    pub body: String,
    pub created_at: i64,
}

impl Comment {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "ticket": self.ticket,
            "author": self.author,
            "body": self.body,
            "created_at": iso(self.created_at),
        })
    }
}

/// A question on the "ask a human" board: an agent's request for a human
/// decision, tied to a ticket. See `store/questions.rs` for the lifecycle.
#[derive(Debug, Clone)]
pub struct Question {
    pub id: String,
    pub project: String,
    pub ticket: String,
    pub asked_by: String,
    /// blocking (parks + resumes the ticket) | advisory (routed + recorded, no
    /// state change — e.g. an epic-level or strategic decision).
    pub mode: String,
    /// confirm | choose | clarify | approve.
    pub kind: String,
    pub title: String,
    pub body: String,
    /// choose-kind options (JSON array of strings); empty otherwise.
    pub options: Vec<String>,
    /// Per-option one-line descriptions, parallel to `options` (empty string
    /// where none). Empty vec when the asker gave plain-string options.
    pub option_notes: Vec<String>,
    /// choose-only: true when several options may be selected at once.
    pub multi: bool,
    /// For a multi choose: the suggested set of options.
    pub recommended_multi: Vec<String>,
    /// The agent's suggested answer (JSON), or Null.
    pub recommended: Value,
    /// A short rationale for the recommendation ("why"), or None.
    pub recommended_note: Option<String>,
    /// How strong the recommendation is, 1-4 (tentative → very strong), or None.
    pub confidence: Option<i64>,
    /// A one-line summary for the inbox list preview, or None (derive from body).
    pub summary: Option<String>,
    /// Routing tags, e.g. ["domain:billing"].
    pub expertise: Vec<String>,
    /// The person this decision is waiting on ([`User::id`]), or None for the open
    /// pool.
    ///
    /// Orthogonal to `expertise`, and both may be set: expertise says what a
    /// qualified answerer must *be*, this says who was actually asked. It is the
    /// difference between "someone who knows billing" and "Ada".
    ///
    /// Assignment is routing — any `human` token may still answer the three
    /// ordinary kinds, so a decision is never stranded because the assignee is
    /// away — with one exception that makes it load-bearing: for an `approve`, a
    /// token bound to this person may answer *in place of* the expertise scope.
    /// See `store::questions::answer_question_inner`.
    pub assignee: Option<String>,
    /// The assignee, resolved for display. Filled by the reads that joined it (see
    /// `Store::resolve_question_people`); `None` on a path that did not, which is
    /// why the wire shape falls back to naming the raw id rather than reporting
    /// the question as unassigned.
    pub assignee_person: Option<User>,
    pub urgency: String,
    /// open | answered | withdrawn | expired.
    pub status: String,
    /// The recorded human answer (JSON), or Null while open.
    pub answer: Value,
    pub answered_by: Option<String>,
    pub answered_at: Option<i64>,
    /// State the ticket was moved to when the answer resolved it, if any.
    pub resolved_to: Option<String>,
    pub expires_at: Option<i64>,
    /// What the expiry sweep does when `expires_at` passes with no answer:
    /// recommended | cancel | escalate. None = leave open (just flag).
    pub on_timeout: Option<String>,
    /// Whose turn it is on the follow-up thread while open: `human` (ready to be
    /// answered) or `agent` (a human asked for more research; the asker owes a
    /// reply). Meaningful only while `status == "open"`.
    pub awaiting: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub version: i64,
}

impl Question {
    /// The assignee as the wire sees it: the resolved person, `null` when the
    /// question is in the open pool, or — if the row names someone a read did not
    /// resolve — an object carrying the id alone.
    ///
    /// That last case degrades to showing an id rather than to `null`, because
    /// `null` would be a lie a reader acts on: it reads as "nobody has this", and
    /// the decision would sit in the open pool of an inbox while its person waited
    /// to be asked.
    fn assignee_json(&self) -> Value {
        match (&self.assignee, &self.assignee_person) {
            (_, Some(person)) => person.to_ref_json(),
            (Some(id), None) => json!({ "id": id, "label": id }),
            (None, None) => Value::Null,
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "project": self.project,
            "ticket": self.ticket,
            "asked_by": self.asked_by,
            "mode": self.mode,
            "kind": self.kind,
            "title": self.title,
            "body": self.body,
            "options": self.options,
            "option_notes": self.option_notes,
            "multi": self.multi,
            "recommended_multi": self.recommended_multi,
            "recommended": self.recommended,
            "recommended_note": self.recommended_note,
            "confidence": self.confidence,
            "summary": self.summary,
            "expertise": self.expertise,
            "assignee": self.assignee_json(),
            "urgency": self.urgency,
            "status": self.status,
            "answer": self.answer,
            "answered_by": self.answered_by,
            "answered_at": self.answered_at.map(iso),
            "resolved_to": self.resolved_to,
            "expires_at": self.expires_at.map(iso),
            "on_timeout": self.on_timeout,
            "awaiting": self.awaiting,
            "created_at": iso(self.created_at),
            "updated_at": iso(self.updated_at),
            "version": self.version,
        })
    }
}

/// One message on a question's follow-up thread (see `store/questions.rs`).
#[derive(Debug, Clone)]
pub struct QuestionMessage {
    pub id: String,
    pub question: String,
    pub author: String,
    /// `human` (asked for more before answering) or `agent` (the asker's reply).
    pub role: String,
    pub body: String,
    pub created_at: i64,
}

impl QuestionMessage {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "question": self.question,
            "author": self.author,
            "role": self.role,
            "body": self.body,
            "created_at": iso(self.created_at),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Event {
    pub seq: i64,
    pub ticket: Option<String>,
    pub project: Option<String>,
    pub actor: String,
    pub kind: String,
    pub payload: Value,
    pub at: i64,
}

impl Event {
    pub fn to_json(&self) -> Value {
        json!({
            "seq": self.seq,
            "ticket": self.ticket,
            "project": self.project,
            "actor": self.actor,
            "kind": self.kind,
            "payload": self.payload,
            "at": iso(self.at),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Lease {
    pub ticket: String,
    pub holder: String,
    pub fence: i64,
    /// None = the lease never expires: an epic claim taken without a TTL.
    pub expires_at: Option<i64>,
    /// True when this lease was **resumed in place** after the holder's previous
    /// one expired — a claim taken in a state the workflow does not mark
    /// claimable, which only the lapsed holder can do and only while nobody else
    /// has claimed since (takomo-jb5i). Renewals and heartbeats leave it false;
    /// they are not the event worth reporting.
    pub resumed: bool,
}

impl Lease {
    pub fn to_json(&self) -> Value {
        let mut out = json!({
            "ticket": self.ticket,
            "holder": self.holder,
            "fence": self.fence,
            // null = never expires (an epic claim without a TTL). Kept as an
            // explicit null rather than an absent field so a caller that reads
            // expires_at sees "none" instead of its own default.
            "expires_at": self.expires_at.map(iso),
        });
        // Additive and only when true: a `"resumed": false` on every lease would
        // read as a field worth checking, when the answer is almost always no.
        if self.resumed {
            out["resumed"] = Value::Bool(true);
        }
        out
    }
}

#[derive(Debug, Clone)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub workflow: crate::workflow::Workflow,
    /// Human-facing language agents should phrase ask-a-human questions in for
    /// this project (e.g. "German"). None = no preference.
    pub question_language: Option<String>,
    /// The project's house style for text agents write — ticket titles/bodies
    /// and human-facing questions (e.g. "Keep it to two sentences; no
    /// marketing voice"). Free-form and advisory. None = no preference.
    pub style_guide: Option<String>,
    /// Default lifetime, in seconds, of an answer link minted for one of this
    /// project's questions. None = unset, and minting falls back to
    /// [`crate::store::DEFAULT_ANSWER_TTL_SECONDS`]. Unlike the two settings
    /// above, this one is enforced rather than advisory: it decides how long a
    /// credential handed to someone outside the org stays live.
    pub answer_link_ttl_seconds: Option<i64>,
    /// The lease a claim on one of this project's tickets gets when it names no
    /// `ttl_seconds`. None = unset, falling back to
    /// [`crate::store::DEFAULT_TTL_SECONDS`]. Enforced, like the setting above —
    /// but about something else entirely: how long a worker may hold a ticket,
    /// not how long a credential handed outside the org stays live.
    pub claim_ttl_seconds: Option<i64>,
    /// The ceiling an explicit `ttl_seconds` on a claim/heartbeat is checked
    /// against. None = unset, falling back to [`crate::store::MAX_TTL_SECONDS`].
    ///
    /// Deliberately unbounded above (takomo-2ztv): a deployment may set whatever
    /// its fleet needs. The cost is that this value *is* the ready queue's
    /// recovery time — the sweeper frees only expired leases, so a crashed worker
    /// parks its ticket for exactly this long.
    pub max_claim_ttl_seconds: Option<i64>,
    /// When this project was archived, or None while it is live.
    ///
    /// A gate rather than a setting: while it is set every write under the
    /// project is refused and its tickets leave the ready queue, while reads are
    /// untouched. Clearing it puts the project back to work unchanged, which is
    /// the whole point — archiving is the reversible thing to reach for when the
    /// alternative is a cascade delete that is not.
    pub archived_at: Option<i64>,
    pub created_at: i64,
}

impl Project {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "workflow": self.workflow.name,
            "question_language": self.question_language,
            "style_guide": self.style_guide,
            "answer_link_ttl_seconds": self.answer_link_ttl_seconds,
            "claim_ttl_seconds": self.claim_ttl_seconds,
            "max_claim_ttl_seconds": self.max_claim_ttl_seconds,
            // Both, deliberately: `archived` is the boolean a client branches on
            // without having to know that "a timestamp is present" means frozen,
            // and `archived_at` is when it happened.
            "archived": self.archived_at.is_some(),
            "archived_at": self.archived_at.map(iso),
            "created_at": iso(self.created_at),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ShareRow {
    pub id: String,
    /// The scope this share grants: [`ShareKind::Project`] (all tickets in
    /// `project`) or [`ShareKind::Subtree`] (`ref_id` root + its full recursive
    /// descendant subtree). Deliberately the enum and not a `String`: the scope
    /// decision is made by matching on it, so there is no unrecognised spelling
    /// that could fall through to the wider query.
    pub kind: ShareKind,
    /// Project id (kind=project) or root ticket id (kind=subtree).
    pub ref_id: String,
    /// Denormalized project the share is scoped to.
    pub project: String,
    pub expires_at: i64,
    pub created_by: String,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
    pub last_used_at: Option<i64>,
}

impl ShareRow {
    /// Public metadata wire shape — never carries the plaintext token or hash.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "kind": self.kind.as_str(),
            "ref": self.ref_id,
            "project": self.project,
            "expires_at": iso(self.expires_at),
            "created_by": self.created_by,
            "created_at": iso(self.created_at),
            "revoked_at": self.revoked_at.map(iso),
            "last_used_at": self.last_used_at.map(iso),
        })
    }
}

/// A per-question answer grant: a write-once, expiring credential that lets an
/// outside party answer exactly one question. See `store/answer_grants.rs`.
#[derive(Debug, Clone)]
pub struct AnswerGrantRow {
    pub id: String,
    pub question: String,
    pub project: String,
    /// Actor recorded as the answerer when this grant is used.
    pub actor: String,
    /// The directory person this link was minted for ([`User::id`]), or None for an
    /// outside expert who is only a free-form `actor`.
    ///
    /// When set, the grant carries that person's *identity*, which is what lets it
    /// answer an `approve` addressed to them. Minting one for someone else is
    /// therefore restricted (`api::questions::mint_answer_link`) — the holder of
    /// the link is whoever holds the string.
    pub user: Option<String>,
    pub expires_at: i64,
    pub created_by: String,
    pub created_at: i64,
    pub used_at: Option<i64>,
    pub revoked_at: Option<i64>,
    pub last_used_at: Option<i64>,
}

impl AnswerGrantRow {
    /// Public metadata — never carries the plaintext token or its hash.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "question": self.question,
            "project": self.project,
            "actor": self.actor,
            "user": self.user,
            "expires_at": iso(self.expires_at),
            "created_by": self.created_by,
            "created_at": iso(self.created_at),
            "used_at": self.used_at.map(iso),
            "revoked_at": self.revoked_at.map(iso),
            "last_used_at": self.last_used_at.map(iso),
        })
    }
}

/// A promotion record: a ticket's work reached a named target/stage. Free-form
/// `target` (e.g. "staging", "production", "published") keeps it domain-agnostic.
#[derive(Debug, Clone)]
pub struct Promotion {
    pub id: String,
    pub ticket: String,
    pub project: String,
    pub target: String,
    pub url: Option<String>,
    pub ref_: Option<String>,
    pub note: Option<String>,
    pub actor: String,
    pub created_at: i64,
}

impl Promotion {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "ticket": self.ticket,
            "project": self.project,
            "target": self.target,
            "url": self.url,
            "ref": self.ref_,
            "note": self.note,
            "actor": self.actor,
            "created_at": iso(self.created_at),
        })
    }
}

/// The derived size/shape of an initiative: how much has accumulated on it. Not
/// stored anywhere — recomputed from `initiative_entries` on every read, so it
/// cannot disagree with the entries it describes.
#[derive(Debug, Clone, Default)]
pub struct InitiativeRollup {
    /// Number of entries.
    pub entries: i64,
    /// Entries that carry attachment bytes (`content_bytes > 0`).
    pub attachments: i64,
    /// Characters of entry text, summed. This is the "how long is this thing"
    /// number a human means — it counts characters, not UTF-8 bytes.
    pub chars: i64,
    /// Total bytes stored: entry text (UTF-8) plus attachment content.
    pub bytes: i64,
    /// Of which attachment content, so a caller can tell a wall of text from a
    /// pile of PDFs without fetching either.
    pub attachment_bytes: i64,
    /// When the most recent entry landed, or None on an initiative with none.
    pub last_entry_at: Option<i64>,
    /// Unresolved notes anchored in the document — a `thread` entry that nothing
    /// supersedes and nobody marked resolved. Someone's unanswered question.
    pub open_notes: i64,
    /// Proposed `view` entries no `decision` has accepted or rejected: wording an
    /// agent is offering that is not live until a person says so.
    pub pending_amendments: i64,
}

impl InitiativeRollup {
    pub fn to_json(&self) -> Value {
        json!({
            "entries": self.entries,
            "attachments": self.attachments,
            "chars": self.chars,
            "bytes": self.bytes,
            "attachment_bytes": self.attachment_bytes,
            // Precomputed rather than left to the caller, because every surface
            // wants it and each would round differently. Two decimals is enough
            // to distinguish a note from a document and never implies more
            // precision than a size in megabytes deserves.
            "megabytes": (self.bytes as f64 / (1024.0 * 1024.0) * 100.0).round() / 100.0,
            "last_entry_at": self.last_entry_at.map(iso),
            // The two attention counts. Named for what is waiting rather than for
            // the entry kinds that produce them, because a caller deciding where
            // to look next does not care that a note is a `thread` row.
            "open_notes": self.open_notes,
            "pending_amendments": self.pending_amendments,
        })
    }
}

/// An initiative: an idea being nurtured, with a collection of research inputs
/// appended to it over time. See `store/initiatives.rs`.
#[derive(Debug, Clone)]
pub struct Initiative {
    pub id: String,
    pub project: String,
    /// Quick title — what the idea is called.
    pub title: String,
    /// A very short description. Deliberately capped well below a ticket body:
    /// the long form belongs in an entry, where it carries provenance.
    pub summary: String,
    /// open | parked | distilled. A plain lifecycle label, not a workflow state.
    pub status: String,
    pub labels: Vec<String>,
    /// Canonical `kind:handle` tag references, same registry as tickets.
    pub tags: Vec<String>,
    pub metadata: Value,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub version: i64,
    /// Derived counts over this initiative's entries.
    pub rollup: InitiativeRollup,
}

impl Initiative {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "project": self.project,
            "title": self.title,
            "summary": self.summary,
            "status": self.status,
            "labels": self.labels,
            "tags": self.tags,
            "metadata": self.metadata,
            "created_by": self.created_by,
            "created_at": iso(self.created_at),
            "updated_at": iso(self.updated_at),
            "version": self.version,
            "rollup": self.rollup.to_json(),
        })
    }
}

/// A mindmap: a tree grown at conversation speed, before any of it is an idea.
///
/// See `store/mindmaps.rs`. What separates it from an [`Initiative`] is not the
/// shape but the stakes: an initiative is nurtured, a mindmap is scratch and
/// deleting one is ordinary. Its `title` is the root everything hangs off.
#[derive(Debug, Clone)]
pub struct Mindmap {
    pub id: String,
    pub project: String,
    /// The root — what the map is about.
    pub title: String,
    pub summary: String,
    /// open | parked | distilled. A label, not a state machine; `distilled` means
    /// its branches have graduated into work.
    pub status: String,
    pub metadata: Value,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub version: i64,
    /// How many nodes hang off it. Derived on read and never stored, for the same
    /// reason an initiative's rollup is: a cached count drifts from the rows it
    /// describes and nothing notices.
    pub nodes: i64,
}

impl Mindmap {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "project": self.project,
            "title": self.title,
            "summary": self.summary,
            "status": self.status,
            "metadata": self.metadata,
            "created_by": self.created_by,
            "created_at": iso(self.created_at),
            "updated_at": iso(self.updated_at),
            "version": self.version,
            "nodes": self.nodes,
        })
    }
}

/// One thought on a mindmap.
#[derive(Debug, Clone)]
pub struct MindmapNode {
    pub id: String,
    pub mindmap: String,
    /// None = a first-ring branch off the root.
    pub parent: Option<String>,
    pub text: String,
    /// Order among siblings; gapped, so inserting between two is one write.
    pub position: i64,
    /// Hand placement, or None to let the layout place it.
    pub x: Option<f64>,
    pub y: Option<f64>,
    /// What this branch became: `epic` or `initiative`, and its id.
    pub promoted_kind: Option<String>,
    pub promoted_id: Option<String>,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl MindmapNode {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "mindmap": self.mindmap,
            "parent": self.parent,
            "text": self.text,
            "position": self.position,
            // Together or not at all: half a coordinate places nothing, so the
            // pair is one fact on the wire rather than two nullable numbers every
            // caller has to recombine.
            "at": match (self.x, self.y) {
                (Some(x), Some(y)) => json!({ "x": x, "y": y }),
                _ => Value::Null,
            },
            "promoted": match (&self.promoted_kind, &self.promoted_id) {
                (Some(kind), Some(id)) => json!({ "kind": kind, "id": id }),
                _ => Value::Null,
            },
            "created_by": self.created_by,
            "created_at": iso(self.created_at),
            "updated_at": iso(self.updated_at),
        })
    }
}

/// One appended contribution to an initiative. Append-only; never edited.
///
/// Carries text, an attachment, or both. The bytes of an attachment are NOT on
/// this struct — `content_bytes` reports the size and the bytes are fetched
/// separately by id, so listing a hundred entries never loads a hundred
/// documents.
#[derive(Debug, Clone)]
pub struct InitiativeEntry {
    pub id: String,
    pub initiative: String,
    pub project: String,
    /// Free-form slug naming the sort of input: `note`, `research`, `feedback`,
    /// `transcript`, `document`, … A new kind is just a new string.
    pub kind: String,
    pub title: Option<String>,
    pub text: String,
    pub mime: Option<String>,
    pub filename: Option<String>,
    pub chars: i64,
    pub text_bytes: i64,
    pub content_bytes: i64,
    /// Where this input came from — an agent, a person, a conversation.
    pub source: String,
    pub source_uri: Option<String>,
    /// When the content originated, as opposed to `created_at` when it landed.
    pub origin_at: Option<i64>,
    pub meta: Value,
    /// The actor that appended it.
    pub author: String,
    pub created_at: i64,
}

impl InitiativeEntry {
    /// Whether this entry carries attachment bytes to fetch.
    pub fn has_content(&self) -> bool {
        self.content_bytes > 0
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "initiative": self.initiative,
            "project": self.project,
            "kind": self.kind,
            "title": self.title,
            "text": self.text,
            "has_content": self.has_content(),
            "mime": self.mime,
            "filename": self.filename,
            "chars": self.chars,
            "bytes": self.text_bytes + self.content_bytes,
            "content_bytes": self.content_bytes,
            "source": self.source,
            "source_uri": self.source_uri,
            "origin_at": self.origin_at.map(iso),
            "meta": self.meta,
            "author": self.author,
            "created_at": iso(self.created_at),
        })
    }
}

/// A project-scoped tag: a named entity of some `kind` (e.g. `person`,
/// `component`, `team`) that tickets can be tagged with by its `handle`. The
/// registry is deliberately generic — a new kind is just a new `kind` string,
/// no schema change — and per-kind attributes live in the free-form `meta`
/// object (a person's email, a component's owner, …). Identity is
/// `(project, kind, handle)`; `label` is the human-facing display name.
#[derive(Debug, Clone)]
pub struct Tag {
    pub id: String,
    pub project: String,
    pub kind: String,
    pub handle: String,
    pub label: String,
    pub meta: Value,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// For a `person:` reference whose handle names somebody in the people
    /// directory: who that is. `None` for every other kind, and for a person
    /// handle nobody has been added under.
    ///
    /// This is what keeps one vocabulary out of two registries. A `person:ada`
    /// tag and the user `ada` are the same person by construction — they share the
    /// handle rule — so the *authority* on their name is the directory, and the tag
    /// row is a reference to it. Lazy-creation writes the handle again as a stub
    /// label; this is what a reader should see instead.
    pub person: Option<TagPerson>,
}

/// The directory person behind a `person:` tag reference. Three facts, not a whole
/// [`User`]: who they are, and whether they can still be handed work.
#[derive(Debug, Clone)]
pub struct TagPerson {
    pub handle: String,
    pub name: Option<String>,
    pub disabled: bool,
}

impl TagPerson {
    /// What to show for this person: their name, falling back to the handle.
    pub fn label(&self) -> String {
        let raw = self
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or(&self.handle);
        raw.chars()
            .map(|c| if display_hostile(c) { '?' } else { c })
            .collect()
    }
}

impl Tag {
    /// Canonical `kind:handle` reference string used on tickets.
    pub fn reference(&self) -> String {
        format!("{}:{}", self.kind, self.handle)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "project": self.project,
            "kind": self.kind,
            "handle": self.handle,
            "ref": self.reference(),
            "label": self.label,
            "meta": self.meta,
            "created_by": self.created_by,
            "created_at": iso(self.created_at),
            "updated_at": iso(self.updated_at),
            // Present only when the handle resolves to somebody in the directory,
            // so a caller can tell "a person we know" from a bare reference.
            "person": self.person.as_ref().map(|p| json!({
                "handle": p.handle,
                "name": p.name,
                "label": p.label(),
                "disabled": p.disabled,
            })),
        })
    }
}

/// A person, global to the server: the directory entry work can be addressed to.
///
/// **A user says who work is waiting on; a scope says what a credential may do.**
/// This is not a login and not a fifth credential type — nothing here
/// authenticates. It is how a question stops meaning "whoever holds
/// `expert:domain:billing`" and starts meaning Ada.
///
/// Global rather than project-scoped, like `tokens` and unlike [`Tag`]: a person
/// is not per-project, so `answered_by` resolves to the same human everywhere.
/// [`UserMembership`] is what bounds where they can be handed work.
///
/// `handle` is validated by the *tag* handle rule (`store::tags::handle_shape_ok`),
/// which is what keeps `person:ada` a legal reference to user `ada`.
///
/// There is no delete. `disabled_at` is a gate in the `projects.archived_at`
/// idiom: a disabled person cannot be newly assigned and cannot exercise
/// assignee authority, while every record naming them keeps resolving — deleting
/// the row would make `answered_by: ada` unreadable after the fact, which is the
/// one thing an audit trail may not do.
#[derive(Debug, Clone)]
pub struct User {
    pub id: String,
    pub handle: String,
    pub name: String,
    pub email: Option<String>,
    pub meta: Value,
    pub disabled_at: Option<i64>,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// Projects this person is a member of, ascending. Filled by the reads that
    /// were asked for it (`get_user`, and `list_users` when the caller wants it);
    /// an empty vec on a read that did not join, which is why the JSON omits the
    /// key rather than claiming "member of nothing".
    pub projects: Vec<String>,
}

impl User {
    /// The canonical `person:<handle>` tag reference for this user, so a caller
    /// tagging a ticket or an initiative with a directory person has one spelling
    /// to use rather than assembling the string itself.
    pub fn tag_reference(&self) -> String {
        format!("person:{}", self.handle)
    }

    pub fn active(&self) -> bool {
        self.disabled_at.is_none()
    }

    /// What to show a human: their name, falling back to the handle. Sanitized
    /// with [`display_hostile`] for the same reason [`OauthConnection::label`] is
    /// — one sink is `takomo user ls` in a terminal, and a name carrying an escape
    /// sequence could forge a row an operator then acts on.
    pub fn label(&self) -> String {
        let raw = if self.name.trim().is_empty() {
            &self.handle
        } else {
            &self.name
        };
        raw.chars()
            .map(|c| if display_hostile(c) { '?' } else { c })
            .collect()
    }

    /// The compact shape embedded wherever a person is *referenced* rather than
    /// listed — a question's `assignee`, `whoami`'s `user`. Enough to render a
    /// name without a second request, and never the membership list.
    pub fn to_ref_json(&self) -> Value {
        json!({
            "id": self.id,
            "handle": self.handle,
            "name": self.name,
            "label": self.label(),
            "ref": self.tag_reference(),
            "disabled": !self.active(),
        })
    }

    pub fn to_json(&self) -> Value {
        let mut out = json!({
            "id": self.id,
            "handle": self.handle,
            "name": self.name,
            "label": self.label(),
            "ref": self.tag_reference(),
            "email": self.email,
            "meta": self.meta,
            "disabled": !self.active(),
            "disabled_at": self.disabled_at.map(iso),
            "created_by": self.created_by,
            "created_at": iso(self.created_at),
            "updated_at": iso(self.updated_at),
        });
        if !self.projects.is_empty() {
            out["projects"] = json!(self.projects);
        }
        out
    }
}

/// One person's membership of one project: who is assignable where.
///
/// Directory scoping, **not** access control. A token's `projects` allowlist
/// remains the only thing deciding what a credential may read or write; this
/// decides who work may be addressed to, and — because a named assignee may
/// answer an `approve` — is a second fence in front of that authority.
#[derive(Debug, Clone)]
pub struct UserMembership {
    pub user: String,
    pub project: String,
    pub added_by: String,
    pub created_at: i64,
}

impl UserMembership {
    pub fn to_json(&self) -> Value {
        json!({
            "user": self.user,
            "project": self.project,
            "added_by": self.added_by,
            "created_at": iso(self.created_at),
        })
    }
}

#[derive(Debug, Clone)]
pub struct TokenRow {
    pub id: String,
    pub actor: String,
    pub scopes: Vec<String>,
    /// None = all projects (`*`).
    pub projects: Option<Vec<String>>,
    pub rate_limit: i64,
    /// The directory person this credential belongs to ([`User::id`]), or None for
    /// a machine token.
    ///
    /// **Binding a token to a user is an authorization act**, not a display
    /// nicety: a named assignee may answer an `approve` question, and this column
    /// is the only proof that the caller *is* that person. So it is set by an
    /// admin at mint time and is never inferred from `actor`, which is a
    /// free-form string anyone with `write` can put anything in.
    pub user: Option<String>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub revoked_at: Option<i64>,
    pub last_used_at: Option<i64>,
    /// Which OAuth connection this token *is*, when it came from the OAuth flow.
    ///
    /// Filled only by the listing query, which is the one place the question gets
    /// asked; every other read of a token row is an authorization decision that has
    /// no use for it. `None` on a hand-minted token, and on every path that does not
    /// join the ledger.
    pub oauth_client: Option<OauthConnection>,
}

/// The connection an OAuth-issued token belongs to, for the listing surfaces.
///
/// Exists because revoking a token now ends a whole connection, which makes
/// picking the wrong row an unrecoverable mistake — and until this was joined
/// through, two connectors approved by the same human were indistinguishable in
/// `takomo token list`: same actor, same scopes, same projects, differing only in
/// id and expiry timestamp.
#[derive(Debug, Clone)]
pub struct OauthConnection {
    pub client_id: String,
    /// The registered `client_name`, absent when the client registered without one
    /// (RFC 7591 makes it optional) or when its registration has since been swept.
    pub client_name: Option<String>,
}

impl OauthConnection {
    /// What to show a human: the client's name, falling back to its id. A person
    /// recognizes "Claude"; nobody recognizes `oc_x3jolbbtodnog1rh`.
    ///
    /// Characters that would forge the display are replaced, not passed through.
    /// This name arrives through an **unauthenticated** registration endpoint and
    /// one of its sinks is a terminal — `takomo token list`, which is the listing an
    /// operator reads to decide which connection to revoke, an act that cannot be
    /// undone. An escape sequence there can erase or overwrite a line, so a forged
    /// row is not cosmetic. `api::oauth::register` refuses such a name outright;
    /// this is the other half of that pair, covering rows stored before that check
    /// existed and whatever writes this column next.
    pub fn label(&self) -> String {
        let raw = self
            .client_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.client_id);
        raw.chars()
            .map(|c| if display_hostile(c) { '?' } else { c })
            .collect()
    }
}

/// Would this character forge or garble a line of text a human is reading?
///
/// Control characters (a newline splitting one row into two, an ANSI escape erasing
/// the row above) and the bidirectional overrides, which reorder what a terminal
/// shows without changing the bytes — the Trojan Source trick, and the same problem
/// wearing different clothes.
///
/// Lives in the store layer because both users need the same answer and only one
/// direction of dependency exists: [`OauthConnection::label`] sanitizes for display,
/// and `api::oauth` refuses on the way in.
pub fn display_hostile(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

impl TokenRow {
    /// Public metadata wire shape — never carries the plaintext or the hash.
    /// `projects` is the string `"*"` (all) or an array of ids, mirroring the
    /// CLI's convention.
    ///
    /// `oauth_client` appears only on a token the OAuth flow issued. Omitted
    /// entirely otherwise rather than emitted as `null`, so a hand-minted token
    /// serializes exactly as it always has.
    pub fn to_json(&self) -> Value {
        let mut out = json!({
            "id": self.id,
            "actor": self.actor,
            "scopes": self.scopes,
            "projects": match &self.projects {
                None => json!("*"),
                Some(list) => json!(list),
            },
            "rate_limit": self.rate_limit,
            "user": self.user,
            "created_at": iso(self.created_at),
            "expires_at": self.expires_at.map(iso),
            "revoked_at": self.revoked_at.map(iso),
            "last_used_at": self.last_used_at.map(iso),
        });
        if let Some(conn) = &self.oauth_client {
            out["oauth_client"] = json!({
                "client_id": conn.client_id,
                "client_name": conn.client_name,
                "label": conn.label(),
            });
        }
        out
    }
}

// ---------------------------------------------------------------------------
// OAuth 2.1 authorization server (src/store/oauth.rs, src/api/oauth.rs).

/// An OAuth client, registered dynamically (RFC 7591) by whichever hosted
/// product is connecting. Always a **public** client: no `client_secret` column,
/// because a client that cannot keep a secret gains nothing from being issued
/// one, and PKCE is what actually binds a code to its requester.
#[derive(Debug, Clone)]
pub struct OauthClient {
    pub client_id: String,
    pub client_name: String,
    /// Exact-match allowlist. A redirect target that is not literally one of
    /// these is refused, which is the whole defense against an open redirect.
    pub redirect_uris: Vec<String>,
    pub created_at: i64,
}

impl OauthClient {
    /// RFC 7591 §3.2.1 client information response.
    pub fn to_json(&self) -> Value {
        json!({
            "client_id": self.client_id,
            "client_id_issued_at": self.created_at / 1000,
            "client_name": self.client_name,
            "redirect_uris": self.redirect_uris,
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        })
    }
}

/// The slice of one takomo token's authority that a human consented to hand a
/// client, snapshotted at consent time.
///
/// Snapshotted rather than looked up later on purpose: the access token minted
/// from it must not silently widen if the consenting token is later given more
/// scopes.
///
/// A snapshot must not survive revocation either, or revocation would be a lie —
/// so `granted_by` is not only a breadcrumb for tracing a connector's authority
/// back to the human who granted it. Every path that mints a credential from this
/// snapshot re-checks that token first, and refuses if it has been revoked or
/// deleted (`GrantRejection::ConsentWithdrawn`). A consenting token that merely
/// *expires* is deliberately not treated that way: a revocation is a decision, an
/// expiry is bookkeeping, and a connected client is meant to stay connected.
#[derive(Debug, Clone)]
pub struct GrantedAccess {
    pub actor: String,
    pub scopes: Vec<String>,
    /// None = all projects (`*`), same convention as [`TokenRow`].
    pub projects: Option<Vec<String>>,
    pub rate_limit: i64,
    /// The directory person the consenting credential belonged to, carried through
    /// the consent snapshot so an OAuth-issued token stays the *same person*.
    ///
    /// Inherited, never granted: consent can narrow scopes but cannot attach an
    /// identity the consenting token did not already have, which is why this is
    /// copied from the token row and never read off the request.
    pub user: Option<String>,
    /// The OAuth scope string exactly as granted (space separated). Echoed back
    /// on the token response, because it may be *narrower* than what the client
    /// asked for — the human can uncheck scopes on the consent screen.
    pub scope: String,
    /// Token id of the credential the human consented with.
    pub granted_by: String,
}

/// What the token endpoint hands back on a successful exchange.
#[derive(Debug, Clone)]
pub struct OauthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub scope: String,
}

/// Why an authorization code or refresh token was refused.
///
/// A store-layer enum rather than an `ApiError`, because the OAuth endpoints are
/// the one place in this codebase that cannot use takomo's own error shape: a
/// client parses RFC 6749's `error` field and nothing else. The mapping to that
/// vocabulary happens at the HTTP edge (`crate::api::oauth`), so the store stays
/// free of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantRejection {
    /// No such code or refresh token.
    Unknown,
    /// Past its expiry.
    Expired,
    /// Already spent (a code) or already rotated (a refresh token). For a refresh
    /// token this also revokes its whole family — see `store::oauth`.
    ///
    /// Reuse, in other words, which is why it is kept distinct from
    /// [`GrantRejection::ConnectionRevoked`]: one says a credential may have been
    /// stolen, the other says it was taken away.
    Replayed,
    /// Presented by a different client than the one it was issued to.
    ClientMismatch,
    /// `redirect_uri` does not match the one the code was bound to.
    RedirectMismatch,
    /// The PKCE `code_verifier` does not hash to the recorded challenge.
    PkceMismatch,
    /// The token the human consented with has been revoked or deleted, so the
    /// snapshot it authorized can mint nothing further. An *expired* consenting
    /// token does not land here — see [`GrantedAccess`].
    ConsentWithdrawn,
    /// This refresh token was revoked without ever having been rotated: the
    /// connection was ended at the server, by `Store::revoke_token` on its derived
    /// token or by reuse detected on a sibling in the same family. Nobody presented
    /// it twice, so it must not be reported as reuse.
    ConnectionRevoked,
}

/// The outcome of a token-endpoint grant: either credentials, or a refusal the
/// edge turns into an RFC 6749 error body.
#[derive(Debug, Clone)]
pub enum OauthExchange {
    Issued(OauthTokens),
    Rejected(GrantRejection),
}

pub const CHECK_LAYERS: [&str; 3] = ["ui", "api", "other"];
pub const CHECK_SEVERITIES: [&str; 3] = ["blocking", "advisory", "low"];
pub const VERIFICATION_LEVELS: [&str; 3] = ["agent", "human", "agent_then_human"];
pub const CASE_VERDICTS: [&str; 4] = ["pass", "fail", "blocked", "unreachable"];

/// What kind of place this is. An enum rather than free text, because a project
/// that grows `prod`, `production` and `Prod` can no longer answer "is it
/// verified in production" — and because a gap on a scratch box and the same gap
/// on production are not the same finding.
pub const ENVIRONMENT_KINDS: [&str; 6] = [
    "local",
    "ephemeral",
    "shared",
    "staging",
    "production",
    "other",
];

/// What is in it, which is what decides whether a case's preconditions can even
/// be met. `unknown` is honest and is the default: an environment nobody has
/// described is not the same as one described as empty.
pub const ENVIRONMENT_DATA_STATES: [&str; 4] = ["seeded", "empty", "production_like", "unknown"];

/// A place a check can be run: a URL, how to bring it up, and what is in it.
///
/// Takomo never touches it. This is a registry an agent reads before running,
/// and the reason it exists is that a verdict with no environment behind it is a
/// claim nobody can reproduce.
#[derive(Debug, Clone)]
pub struct Environment {
    pub id: String,
    pub project: String,
    /// The handle an agent types. Immutable once created.
    pub slug: String,
    pub name: String,
    pub kind: String,
    pub base_url: Option<String>,
    pub bring_up: String,
    pub teardown: String,
    pub data_state: String,
    /// Advisory only — Takomo executes nothing and enforces nothing here.
    pub writable: bool,
    /// A POINTER to where a credential lives. Never a credential.
    pub credentials_hint: Option<String>,
    pub notes: String,
    pub metadata: Value,
    pub version: i64,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived_at: Option<i64>,
}

impl Environment {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "project": self.project,
            "slug": self.slug,
            "name": self.name,
            "kind": self.kind,
            "base_url": self.base_url,
            "bring_up": self.bring_up,
            "teardown": self.teardown,
            "data_state": self.data_state,
            "writable": self.writable,
            "credentials_hint": self.credentials_hint,
            "notes": self.notes,
            "metadata": self.metadata,
            "version": self.version,
            "created_by": self.created_by,
            "created_at": iso(self.created_at),
            "updated_at": iso(self.updated_at),
            "archived_at": self.archived_at.map(iso),
        })
    }
}

/// An ordered marker in a project's release history, pushed by the agent that
/// merged the work. `seq` is monotonic per project so a release-count expiry
/// ("retest every 5 releases") is arithmetic.
#[derive(Debug, Clone)]
pub struct Release {
    pub id: String,
    pub project: String,
    pub reference: String,
    pub seq: i64,
    pub note: Option<String>,
    pub pushed_by: String,
    pub created_at: i64,
    pub path_count: i64,
    pub orphan_globs: Vec<String>,
}

impl Release {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "project": self.project,
            "ref": self.reference,
            "seq": self.seq,
            "note": self.note,
            "pushed_by": self.pushed_by,
            "created_at": iso(self.created_at),
            "touched_paths": self.path_count,
            "orphan_globs": self.orphan_globs,
        })
    }
}

/// What a release push invalidated. Returned to the pusher so the agent learns
/// immediately what it just made stale, without a second request.
#[derive(Debug, Clone, Default)]
pub struct ReleaseImpact {
    pub stale_cases: i64,
    pub stale_checks: Vec<String>,
    pub expired_checks: Vec<String>,
    pub orphaned_checks: Vec<String>,
}

impl ReleaseImpact {
    pub fn to_json(&self) -> Value {
        json!({
            "stale_cases": self.stale_cases,
            "stale_checks": self.stale_checks,
            "expired_checks": self.expired_checks,
            "orphaned_checks": self.orphaned_checks,
        })
    }
}

/// How many of a check's live cases sit in each state. Counted, never derived
/// twice: `total` is the sum of the rest.
#[derive(Debug, Clone, Default)]
pub struct CheckCounts {
    pub total: i64,
    pub approved: i64,
    pub verified: i64,
    pub stale: i64,
    pub failed: i64,
    pub unreachable: i64,
    pub never: i64,
}

impl CheckCounts {
    pub fn to_json(&self) -> Value {
        json!({
            "total": self.total,
            "approved": self.approved,
            "verified": self.verified,
            "stale": self.stale,
            "failed": self.failed,
            "unreachable": self.unreachable,
            "never": self.never,
        })
    }

    pub fn add(&mut self, state: &str) {
        self.total += 1;
        match state {
            "approved" => self.approved += 1,
            "verified" => self.verified += 1,
            "stale" => self.stale += 1,
            "failed" => self.failed += 1,
            "unreachable" => self.unreachable += 1,
            _ => self.never += 1,
        }
    }
}

/// The policy actually in force for a check, after resolving project → epic →
/// check. `verification_from` / `expiry_from` name the level that supplied the
/// value so a reader can see why, which is the whole point of an inherited
/// setting.
#[derive(Debug, Clone)]
pub struct ResolvedPolicy {
    pub verification: String,
    pub verification_from: String,
    pub expiry_days: Option<i64>,
    pub expiry_releases: Option<i64>,
    pub expiry_from: String,
}

impl ResolvedPolicy {
    pub fn to_json(&self) -> Value {
        json!({
            "verification": self.verification,
            "verification_from": self.verification_from,
            "expiry_days": self.expiry_days,
            "expiry_releases": self.expiry_releases,
            "expiry_from": self.expiry_from,
        })
    }

    /// Does clearing a case under this policy need a human?
    pub fn needs_human(&self) -> bool {
        self.verification == "human" || self.verification == "agent_then_human"
    }
}

/// One action with one entry precondition at one layer. `body` is free-form
/// prose an agent or a human follows; there is deliberately no step model.
#[derive(Debug, Clone)]
pub struct Check {
    pub id: String,
    pub project: String,
    pub epic: Option<String>,
    /// The initiative whose conversation agreed this check should exist.
    /// Direct, not derived through the epic: the agreement usually predates any
    /// epic, so deriving it would make the link unstateable when it is made.
    pub initiative: Option<String>,
    pub title: String,
    pub body: String,
    pub precondition: String,
    pub layer: String,
    pub severity: String,
    pub verification: Option<String>,
    pub expiry_days: Option<i64>,
    pub expiry_releases: Option<i64>,
    pub cost_agent_minutes: Option<i64>,
    pub cost_human_minutes: Option<i64>,
    pub metadata: Value,
    pub version: i64,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived_at: Option<i64>,
    pub globs: Vec<String>,
    pub counts: CheckCounts,
    /// Globs that matched nothing in the most recent release — coverage claimed
    /// over code that is not there.
    pub orphan_globs: Vec<String>,
    pub policy: Option<ResolvedPolicy>,
}

impl Check {
    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "id": self.id,
            "project": self.project,
            "epic": self.epic,
            "initiative": self.initiative,
            "title": self.title,
            "body": self.body,
            "precondition": self.precondition,
            "layer": self.layer,
            "severity": self.severity,
            "verification": self.verification,
            "expiry_days": self.expiry_days,
            "expiry_releases": self.expiry_releases,
            "cost_agent_minutes": self.cost_agent_minutes,
            "cost_human_minutes": self.cost_human_minutes,
            "metadata": self.metadata,
            "version": self.version,
            "globs": self.globs,
            "orphan_globs": self.orphan_globs,
            "cases": self.counts.to_json(),
            "created_by": self.created_by,
            "created_at": iso(self.created_at),
            "updated_at": iso(self.updated_at),
            "archived_at": self.archived_at.map(iso),
        });
        if let Some(p) = &self.policy {
            v["policy"] = p.to_json();
        }
        v
    }
}

/// A check crossed with one parameter assignment: the unit that actually gets
/// executed. `key` is stable across regeneration so history survives.
#[derive(Debug, Clone)]
pub struct Case {
    pub id: String,
    pub check: String,
    pub key: String,
    pub label: String,
    pub assignment: Value,
    pub seeded: bool,
    pub agent_verdict: Option<String>,
    pub agent_at: Option<i64>,
    pub agent_by: Option<String>,
    pub agent_release: Option<String>,
    pub human_verdict: Option<String>,
    pub human_at: Option<i64>,
    pub human_by: Option<String>,
    pub human_release: Option<String>,
    pub stale_since: Option<String>,
    pub retired_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Case {
    /// The one word that describes where this case stands.
    ///
    /// `stale` outranks `unreachable` deliberately: once the claimed code moved,
    /// an earlier "the interface cannot reach this" is a statement about code
    /// that no longer exists, so it has to be re-established rather than trusted.
    pub fn state(&self) -> &'static str {
        if self.retired_at.is_some() {
            return "retired";
        }
        if self.stale_since.is_some() {
            return "stale";
        }
        if self.agent_verdict.as_deref() == Some("unreachable")
            || self.human_verdict.as_deref() == Some("unreachable")
        {
            return "unreachable";
        }
        if self.agent_verdict.as_deref() == Some("fail")
            || self.human_verdict.as_deref() == Some("fail")
        {
            return "failed";
        }
        if self.human_verdict.as_deref() == Some("pass") {
            return "approved";
        }
        if self.agent_verdict.as_deref() == Some("pass") {
            return "verified";
        }
        "never"
    }

    /// Is this case cleared under `policy`? `agent_then_human` needs both facts,
    /// which is exactly why they are stored separately.
    pub fn satisfies(&self, policy: &ResolvedPolicy) -> bool {
        if self.state() == "unreachable" {
            return true;
        }
        match policy.verification.as_str() {
            "human" => self.human_verdict.as_deref() == Some("pass"),
            "agent_then_human" => {
                self.agent_verdict.as_deref() == Some("pass")
                    && self.human_verdict.as_deref() == Some("pass")
            }
            _ => {
                self.agent_verdict.as_deref() == Some("pass")
                    || self.human_verdict.as_deref() == Some("pass")
            }
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "check": self.check,
            "key": self.key,
            "label": self.label,
            "assignment": self.assignment,
            "seeded": self.seeded,
            "state": self.state(),
            "agent": {
                "verdict": self.agent_verdict,
                "at": self.agent_at.map(iso),
                "by": self.agent_by,
                "release": self.agent_release,
            },
            "human": {
                "verdict": self.human_verdict,
                "at": self.human_at.map(iso),
                "by": self.human_by,
                "release": self.human_release,
            },
            "stale_since": self.stale_since,
            "retired_at": self.retired_at.map(iso),
            "created_at": iso(self.created_at),
            "updated_at": iso(self.updated_at),
        })
    }
}

/// One recorded verdict, append-only.
#[derive(Debug, Clone)]
pub struct CaseVerdict {
    pub id: String,
    pub case_id: String,
    pub actor_kind: String,
    pub actor: String,
    pub verdict: String,
    pub note: Option<String>,
    pub release: Option<String>,
    pub at: i64,
}

impl CaseVerdict {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "case": self.case_id,
            "actor_kind": self.actor_kind,
            "actor": self.actor,
            "verdict": self.verdict,
            "note": self.note,
            "release": self.release,
            "at": iso(self.at),
        })
    }
}

/// A recurrence rule that materializes ordinary tickets. See
/// [`crate::store::schedules`] and `spec/schedule-format.md`.
#[derive(Debug, Clone)]
pub struct Schedule {
    pub id: String,
    pub project: String,
    pub name: String,
    /// The parsed cadence, or `None` when the stored JSON no longer parses — a
    /// corrupt row rather than a schedule with default behaviour, which is why
    /// this is an Option and not a fallback.
    pub cadence: Option<crate::schedule::Cadence>,
    /// The cadence exactly as stored, so a corrupt row can be reported verbatim
    /// instead of guessed at.
    pub cadence_raw: String,
    pub template: Value,
    /// pending | active | paused | rejected | retired.
    pub status: String,
    /// The agent that proposed it, when it arrived over MCP and needed approval.
    pub proposed_by: Option<String>,
    pub rationale: Option<String>,
    /// The next slot to fire. NULL unless `status = active`, which is what makes
    /// a pending or paused schedule inert by construction: the sweep's index
    /// cannot see it.
    pub next_slot: Option<i64>,
    /// The interval anchor and the earliest slot.
    pub starts_at: i64,
    pub ends_at: Option<i64>,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub version: i64,
}

impl Schedule {
    pub fn to_json(&self, upcoming: &[i64]) -> Value {
        json!({
            "id": self.id,
            "project": self.project,
            "name": self.name,
            "cadence": self.cadence,
            "template": self.template,
            "status": self.status,
            "proposed_by": self.proposed_by,
            "rationale": self.rationale,
            "next_slot": self.next_slot.map(iso),
            "starts_at": iso(self.starts_at),
            "ends_at": self.ends_at.map(iso),
            "upcoming": upcoming.iter().copied().map(iso).collect::<Vec<_>>(),
            "created_by": self.created_by,
            "created_at": iso(self.created_at),
            "updated_at": iso(self.updated_at),
            "version": self.version,
        })
    }
}

/// One occurrence of a schedule, with its outcome derived from the ticket.
#[derive(Debug, Clone)]
pub struct ScheduleOccurrence {
    pub ticket: String,
    pub slot: i64,
    pub expires_at: Option<i64>,
    pub title: String,
    pub state: String,
    pub state_category: String,
    /// done | open | not_fulfilled.
    pub outcome: String,
    /// Who holds or last held the lease — evidence for the outcome, not part of
    /// it.
    pub claimed_by: Option<String>,
    pub archived_at: Option<String>,
}

impl ScheduleOccurrence {
    pub fn to_json(&self) -> Value {
        json!({
            "ticket": self.ticket,
            "slot": iso(self.slot),
            "expires_at": self.expires_at.map(iso),
            "title": self.title,
            "state": self.state,
            "state_category": self.state_category,
            "outcome": self.outcome,
            "claimed_by": self.claimed_by,
            "archived_at": self.archived_at,
        })
    }
}

/// One entry in the workflow library: a named state machine, plus where its
/// nodes sit in the editor.
///
/// `workflow` is a `Value` rather than a `Workflow` on purpose. This row is
/// storage, and a document written by an older or newer binary must still be
/// listable — parsing it into the typed struct here would make one unreadable
/// row take down the whole list. The typed parse happens where it matters, on
/// the way IN (validation) and on the way OUT into a project (applying).
pub struct WorkflowEntry {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub workflow: Value,
    /// Node positions for the editor. Outside the workflow document by
    /// necessity: `Workflow` is `deny_unknown_fields`.
    pub layout: Option<Value>,
    /// Ships with the server: reseeded on every start, so not editable here.
    pub builtin: bool,
    pub created_at: i64,
    pub created_by: String,
    pub updated_at: i64,
}

impl WorkflowEntry {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "description": self.description,
            "workflow": self.workflow,
            "layout": self.layout,
            "builtin": self.builtin,
            "created_at": iso(self.created_at),
            "created_by": self.created_by,
            "updated_at": iso(self.updated_at),
        })
    }
}
