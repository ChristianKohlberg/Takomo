//! Row models and wire (JSON) shapes.

use crate::ids::iso;
use serde_json::{json, Value};

pub const TICKET_TYPES: [&str; 4] = ["epic", "task", "bug", "spike"];
pub const PRIORITIES: [&str; 4] = ["critical", "high", "normal", "low"];

pub const MAX_TITLE: usize = 300;
pub const MAX_BODY: usize = 131_072;
pub const MAX_COMMENT: usize = 65_536;
pub const MAX_METADATA: usize = 65_536;

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
    pub metadata: Value,
    pub links: Value,
    pub blocked_by: Vec<String>,
    pub claim_holder: Option<String>,
    pub claim_expires_at: Option<i64>,
    pub fence_seq: i64,
    pub version: i64,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// ISO timestamp when the ticket was archived, or None when active.
    /// Archived tickets are hidden from default list/ready/board/metrics views.
    pub archived_at: Option<String>,
}

impl Ticket {
    /// Whether the claim is active at `now` (expired leases read as unclaimed).
    pub fn active_claim(&self, now: i64) -> Option<(&str, i64)> {
        match (&self.claim_holder, self.claim_expires_at) {
            (Some(h), Some(exp)) if exp > now => Some((h.as_str(), exp)),
            _ => None,
        }
    }

    pub fn to_json(&self, now: i64) -> Value {
        let claim = self
            .active_claim(now)
            .map(|(h, exp)| json!({ "holder": h, "expires_at": iso(exp) }))
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
            "metadata": self.metadata,
            "links": self.links,
            "blocked_by": self.blocked_by,
            "claim": claim,
            "version": self.version,
            "created_by": self.created_by,
            "created_at": iso(self.created_at),
            "updated_at": iso(self.updated_at),
            "archived_at": self.archived_at,
        })
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
    pub expires_at: i64,
}

impl Lease {
    pub fn to_json(&self) -> Value {
        json!({
            "ticket": self.ticket,
            "holder": self.holder,
            "fence": self.fence,
            "expires_at": iso(self.expires_at),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub workflow: crate::workflow::Workflow,
    pub created_at: i64,
}

impl Project {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "workflow": self.workflow.name,
            "created_at": iso(self.created_at),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ShareRow {
    pub id: String,
    /// "project" (all tickets in `project`) or "subtree" (`ref_id` root + its
    /// full recursive descendant subtree).
    pub kind: String,
    /// Project id (kind=project) or root ticket id (kind=subtree).
    pub ref_id: String,
    /// Denormalized project the share is scoped to.
    pub project: String,
    pub expires_at: i64,
    pub created_by: String,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
}

impl ShareRow {
    /// Public metadata wire shape — never carries the plaintext token or hash.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "kind": self.kind,
            "ref": self.ref_id,
            "project": self.project,
            "expires_at": iso(self.expires_at),
            "created_by": self.created_by,
            "created_at": iso(self.created_at),
            "revoked_at": self.revoked_at.map(iso),
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
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub revoked_at: Option<i64>,
    pub last_used_at: Option<i64>,
}

impl TokenRow {
    /// Public metadata wire shape — never carries the plaintext or the hash.
    /// `projects` is the string `"*"` (all) or an array of ids, mirroring the
    /// CLI's convention.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "actor": self.actor,
            "scopes": self.scopes,
            "projects": match &self.projects {
                None => json!("*"),
                Some(list) => json!(list),
            },
            "rate_limit": self.rate_limit,
            "created_at": iso(self.created_at),
            "expires_at": self.expires_at.map(iso),
            "revoked_at": self.revoked_at.map(iso),
            "last_used_at": self.last_used_at.map(iso),
        })
    }
}
