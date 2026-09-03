//! What an agent may do to a document: read it, and **propose** changes to it.
//!
//! This is the half of the design KONZEPT is actually about — *„Der Agent
//! schlägt vor, der Mensch entscheidet."* Everything here is shaped by two rules
//! that are not negotiable, and one consequence of them.
//!
//! ## An agent returns operations, never a document
//!
//! The alternative — read the document, ask a model, write the answer back — is
//! the failure the whole surface exists to remove: it clobbers whatever was typed
//! during the seconds the model spent thinking, and it turns a one-word fix into
//! a whole-document diff nobody can review.
//!
//! So an agent addresses **blocks by id**:
//!
//! ```json
//! [{"op": "replace",      "id": "blk_7f3a", "markdown": "## Pricing\n…"},
//!  {"op": "insert_after", "id": "blk_7f3a", "markdown": "…"},
//!  {"op": "delete",       "id": "blk_9c1e"}]
//! ```
//!
//! Blocks it does not name are untouched, so a human editing three paragraphs
//! away keeps their words. That is a property of the op vocabulary rather than of
//! the prompt, which is the point: telling a model to stay in its lane is not the
//! same as knowing it did.
//!
//! ## Nothing an agent writes is live text
//!
//! A proposal is stored **beside** the prose, in a `proposals` map in the same
//! Y.Doc, and a person accepts or rejects it. Two consequences worth stating:
//!
//! - It is in the CRDT, so every connected peer sees it the moment it lands and
//!   it survives a disconnect. A proposal held server-side until somebody opened
//!   the page would be a second source of truth about the same document.
//! - **Applying it is the browser's job, not this module's.** Turning markdown
//!   into ProseMirror nodes means knowing the editor's exact schema, and the
//!   editor is the only thing that does. Rust writing nodes it half-understands
//!   is how a shared document gets quietly corrupted.
//!
//! So Rust READS the document (walking the CRDT is unambiguous) and WRITES only
//! the proposal record. The asymmetry is deliberate.
//!
//! ## The scope is enforced, not requested
//!
//! A run may be scoped to a set of block ids. That scope is stated in the prompt
//! *and* checked here: an op outside it is dropped and reported. Somebody who
//! selected one paragraph expects the rest to be untouched, and a model's
//! agreement to that is not evidence.

use crate::error::{ApiError, ApiResult};
use serde_json::{json, Value};
use yrs::types::xml::XmlOut;
use yrs::{Any, GetString, Map, ReadTxn, Transact, Xml, XmlFragment};

/// The Y.Doc key the editor binds its prose to. Must match the `field` given to
/// Tiptap's `Collaboration` extension in `web/src/pages/documents/Editor.tsx` —
/// they are the same string on two sides of a wire.
pub const PROSE_FIELD: &str = "prose";

/// The Y.Doc key proposals live under, beside the prose rather than inside it.
pub const PROPOSALS_FIELD: &str = "proposals";

/// Cap on ops in one proposal. A proposal is a reviewable unit; past this it is
/// a rewrite wearing a diff's clothes, and nobody reads it.
pub const MAX_OPS: usize = 64;

/// Cap on one op's markdown.
pub const MAX_OP_MARKDOWN: usize = 32 * 1024;

/// Cap on live proposals in a document, so an agent in a loop cannot fill it.
pub const MAX_PENDING_PROPOSALS: usize = 50;

/// One block of the document, as an agent sees it.
pub struct Block {
    pub id: String,
    /// The ProseMirror node name: `paragraph`, `heading`, `bulletList`, …
    pub kind: String,
    pub level: Option<i64>,
    /// The block's text, with nested structure flattened.
    pub text: String,
    /// One entry per direct child element — the list items of a list.
    ///
    /// Kept apart from `text` because a list is the one block whose meaning
    /// lives in its boundaries: flattened, "Milch Brot" is a different statement
    /// from two items, and an agent asked to add one would have nothing to
    /// append to.
    pub items: Vec<String>,
}

/// Read the document as markdown annotated with block ids.
///
/// The annotation is what makes the reply addressable:
///
/// ```text
/// <!-- blk_7f3a -->
/// ## Pricing
/// Our current tiers are…
/// ```
///
/// Only TOP-LEVEL nodes carry an id, matching `web/src/lib/block-id.ts`: a
/// paragraph inside a list item is part of that block, not a block of its own.
pub fn read_blocks<T: ReadTxn>(txn: &T, frag: &yrs::XmlFragmentRef) -> Vec<Block> {
    let mut out = Vec::new();
    for node in frag.children(txn) {
        let XmlOut::Element(el) = node else { continue };
        let items: Vec<String> = el
            .children(txn)
            .filter_map(|c| match c {
                XmlOut::Element(child) => Some(element_text(txn, &child)),
                _ => None,
            })
            .collect();
        out.push(Block {
            id: attr_string(txn, &el, "id").unwrap_or_default(),
            kind: el.tag().to_string(),
            level: attr_int(txn, &el, "level"),
            text: element_text(txn, &el),
            items,
        });
    }
    out
}

/// The text inside an element, with nested structure flattened.
///
/// NOT `get_string`, which serializes the element back to XML tags and would
/// hand an agent `<paragraph id="blk_x">…</paragraph>` as if it were prose. That
/// is what shipped in the first draft, and the test caught it.
fn element_text<T: ReadTxn>(txn: &T, el: &yrs::XmlElementRef) -> String {
    let mut out = String::new();
    for child in el.children(txn) {
        match child {
            XmlOut::Text(t) => out.push_str(&t.get_string(txn)),
            XmlOut::Element(e) => out.push_str(&element_text(txn, &e)),
            XmlOut::Fragment(_) => {}
        }
    }
    out
}

fn attr_string<T: ReadTxn>(txn: &T, el: &yrs::XmlElementRef, key: &str) -> Option<String> {
    match el.get_attribute(txn, key)? {
        Out::Any(Any::String(s)) => Some(s.to_string()),
        Out::Any(other) => Some(other.to_string()),
        _ => None,
    }
}

/// An integer attribute, accepting the number `y-prosemirror` writes and the
/// string a hand-built document might.
///
/// `sync-plugin.js` passes ProseMirror's attribute value through untouched, so a
/// heading's `level` arrives as a number. Tolerating the string form as well
/// costs one branch and means a heading does not silently flatten to `#` if that
/// ever changes.
fn attr_int<T: ReadTxn>(txn: &T, el: &yrs::XmlElementRef, key: &str) -> Option<i64> {
    match el.get_attribute(txn, key)? {
        Out::Any(Any::Number(n)) => Some(n as i64),
        Out::Any(Any::BigInt(n)) => Some(n),
        Out::Any(Any::String(s)) => s.parse().ok(),
        _ => None,
    }
}

use yrs::Out;

/// Render the blocks the way an agent reads them.
pub fn annotate(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        if !b.id.is_empty() {
            out.push_str(&format!("<!-- {} -->\n", b.id));
        }
        out.push_str(&markdown_for(b));
        out.push_str("\n\n");
    }
    out.trim_end().to_string()
}

fn markdown_for(b: &Block) -> String {
    match b.kind.as_str() {
        "heading" => {
            let level = b.level.unwrap_or(1).clamp(1, 6) as usize;
            format!("{} {}", "#".repeat(level), b.text)
        }
        "codeBlock" => format!("```\n{}\n```", b.text),
        "blockquote" => b
            .text
            .lines()
            .map(|l| format!("> {l}"))
            .collect::<Vec<_>>()
            .join("\n"),
        "horizontalRule" => "---".to_string(),
        "bulletList" => b
            .items
            .iter()
            .map(|i| format!("- {i}"))
            .collect::<Vec<_>>()
            .join("\n"),
        "orderedList" => b
            .items
            .iter()
            .enumerate()
            .map(|(n, i)| format!("{}. {i}", n + 1))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => b.text.clone(),
    }
}

/// One operation against a block.
#[derive(Debug, Clone)]
pub struct Op {
    pub kind: OpKind,
    pub id: String,
    pub markdown: String,
    /// One clause saying WHY, shown next to this change in the review panel.
    /// Optional: an agent that omits it still gets its proposal offered.
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpKind {
    Replace,
    InsertAfter,
    Delete,
}

impl OpKind {
    fn as_str(&self) -> &'static str {
        match self {
            OpKind::Replace => "replace",
            OpKind::InsertAfter => "insert_after",
            OpKind::Delete => "delete",
        }
    }
}

/// What came of validating a batch of ops: the ones that will be offered, and
/// prose about each one that will not.
pub struct Validated {
    pub ops: Vec<Op>,
    /// One sentence per dropped op, returned to the agent so it can correct
    /// itself. Dropping silently would let a model believe it had changed
    /// something it had not.
    pub skipped: Vec<String>,
}

/// Parse and check ops against the document as it stands now.
///
/// An op naming a block that no longer exists is **skipped and reported**, not
/// an error: a human may legitimately have deleted that paragraph while the
/// model was thinking, and failing the whole run over it would make every
/// concurrent edit a lost agent run.
///
/// `read_tool` is the MCP tool that re-reads THIS surface. It is a parameter
/// rather than a constant because two surfaces share this function: a plan
/// section is read with `takomo_plan_read`, and telling its agent to call
/// `takomo_document_read` sends it to a tool that cannot see the section it is
/// being asked to look at again. A remedy that names the wrong tool is worse
/// than none, because the agent will follow it.
pub fn validate_ops(
    raw: &Value,
    blocks: &[Block],
    scope: Option<&[String]>,
    read_tool: &str,
) -> ApiResult<Validated> {
    let list = raw.as_array().ok_or_else(|| {
        ApiError::validation(
            "validation.document_ops",
            "`ops` must be an array of operations.".to_string(),
        )
        .remedy(
            "Send [{\"op\":\"replace\",\"id\":\"blk_…\",\"markdown\":\"…\"}]. Address blocks \
             by the id in the `<!-- blk_… -->` comment above them; never send a whole \
             document."
                .to_string(),
        )
    })?;

    if list.len() > MAX_OPS {
        return Err(ApiError::validation(
            "validation.document_ops",
            format!(
                "A proposal carries {} operations; the maximum is {MAX_OPS}.",
                list.len()
            ),
        )
        .remedy(
            "Split it. A proposal is something a person reads and decides on in one \
             sitting — past this it is a rewrite, and nobody reviews those."
                .to_string(),
        ));
    }

    let known: std::collections::HashSet<&str> = blocks.iter().map(|b| b.id.as_str()).collect();
    let mut ops = Vec::new();
    let mut skipped = Vec::new();

    for (i, item) in list.iter().enumerate() {
        let obj = match item.as_object() {
            Some(o) => o,
            None => {
                skipped.push(format!("op {i} is not an object"));
                continue;
            }
        };
        let kind = match obj.get("op").and_then(Value::as_str) {
            Some("replace") => OpKind::Replace,
            Some("insert_after") => OpKind::InsertAfter,
            Some("delete") => OpKind::Delete,
            other => {
                skipped.push(format!(
                    "op {i} has op={other:?}; expected replace, insert_after or delete"
                ));
                continue;
            }
        };
        let id = match obj.get("id").and_then(Value::as_str) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => {
                skipped.push(format!("op {i} names no block id"));
                continue;
            }
        };
        if !known.contains(id.as_str()) {
            skipped.push(format!(
                "op {i} targets '{id}', which is not in the document any more — \
                 somebody may have deleted it while you were working"
            ));
            continue;
        }
        // The scope check. Enforced here rather than trusted to the prompt,
        // because a reader who selected one paragraph expects the rest untouched
        // and a model's agreement to that is not evidence.
        if let Some(scope) = scope {
            if !scope.iter().any(|s| s == &id) {
                skipped.push(format!(
                    "op {i} targets '{id}', which is outside the requested scope"
                ));
                continue;
            }
        }
        let markdown = obj
            .get("markdown")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if kind != OpKind::Delete && markdown.trim().is_empty() {
            skipped.push(format!("op {i} carries no markdown"));
            continue;
        }
        // A `replace` that changes nothing.
        //
        // Not hypothetical, and not a code bug: a model asked to add an open
        // question answered with a `replace` carrying the block's existing text
        // verbatim, while its summary described the question it had not written.
        // Stored faithfully that is a change a reviewer reads, accepts, and gets
        // nothing from — with a summary that told them otherwise. Comparing
        // against the same rendering `annotate` produced is what makes the check
        // exact rather than approximate.
        if kind == OpKind::Replace {
            if let Some(block) = blocks.iter().find(|b| b.id == id) {
                if markdown.trim() == markdown_for(block).trim() {
                    skipped.push(format!(
                        "op {i} would replace '{id}' with the text it already has — no change"
                    ));
                    continue;
                }
            }
        }
        if markdown.len() > MAX_OP_MARKDOWN {
            skipped.push(format!(
                "op {i} carries {} bytes of markdown; the maximum is {MAX_OP_MARKDOWN}",
                markdown.len()
            ));
            continue;
        }
        let rationale = obj
            .get("rationale")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .chars()
            .take(400)
            .collect();
        ops.push(Op {
            kind,
            id,
            markdown,
            rationale,
        });
    }

    if ops.is_empty() {
        let all_no_ops = !list.is_empty() && skipped.iter().all(|s| s.ends_with("no change"));
        if all_no_ops {
            return Err(ApiError::validation(
                "validation.document_unchanged",
                "Every proposed change was the text the document already has, so there is \
                 nothing to decide."
                    .to_string(),
            )
            .remedy(
                "If the document already satisfies the request, say so instead of \
                 proposing a replacement that changes nothing."
                    .to_string(),
            ));
        }
        return Err(ApiError::validation(
            "validation.document_ops",
            format!(
                "None of the {} operations could be applied: {}",
                list.len(),
                if skipped.is_empty() {
                    "the list was empty".to_string()
                } else {
                    skipped.join("; ")
                }
            ),
        )
        .remedy(format!(
            "Re-read it with {read_tool} — the block ids may have moved on — and \
             address only the blocks it lists."
        )));
    }

    Ok(Validated { ops, skipped })
}

/// A proposal record as stored: one JSON string per map entry.
///
/// Matched rather than `to_string()`d, because `Out`'s Display would happily
/// stringify a non-string value into something that then fails to parse further
/// down, where the cause is much harder to see.
fn record_json(value: &Out) -> Option<Value> {
    match value {
        Out::Any(Any::String(s)) => serde_json::from_str(s).ok(),
        _ => None,
    }
}

/// Write a proposal into the document's `proposals` map.
///
/// Returns the proposal id. The record is JSON in a `Y.Map` entry rather than a
/// nested Yjs structure: nothing merges *within* a proposal — it is written once
/// and then only its status changes — so the structure would buy concurrency
/// nobody needs and cost a schema both sides must agree on.
#[allow(clippy::too_many_arguments)]
pub fn write_proposal(
    doc: &yrs::Doc,
    // The section this is about, when the document is a PLAN rather than a
    // standalone document. `None` for a document, whose proposals are about the
    // whole of it.
    node: Option<&str>,
    author: &str,
    instruction: &str,
    summary: &str,
    ops: &[Op],
    skipped: &[String],
    now: i64,
) -> ApiResult<String> {
    let id = format!("prop-{}", crate::ids::proposal_suffix());
    let map = doc.get_or_insert_map(PROPOSALS_FIELD);

    let mut txn = doc.transact_mut();

    let pending = map
        .iter(&txn)
        .filter(|(_, v)| {
            record_json(v)
                .and_then(|p| {
                    p.get("status")
                        .and_then(Value::as_str)
                        .map(|s| s == "pending")
                })
                .unwrap_or(false)
        })
        .count();
    if pending >= MAX_PENDING_PROPOSALS {
        return Err(ApiError::validation(
            "validation.document_proposals",
            format!(
                "This document already has {pending} undecided proposals; the maximum \
                 is {MAX_PENDING_PROPOSALS}."
            ),
        )
        .remedy(
            "Somebody has to accept or reject the ones already offered. A pile of \
             proposals nobody decides on is the state this cap exists to make visible."
                .to_string(),
        ));
    }

    let record = json!({
        "id": id,
        // Which section, when the document is a plan. A standalone document has
        // no sections, so this is null there.
        "node": node,
        "status": "pending",
        "author": author,
        "instruction": instruction,
        "summary": summary,
        "created_at": now,
        "skipped": skipped,
        "ops": ops.iter().map(|o| json!({
            "op": o.kind.as_str(),
            "id": o.id,
            "markdown": o.markdown,
            "rationale": o.rationale,
        })).collect::<Vec<_>>(),
    });

    map.insert(&mut txn, id.clone(), record.to_string());
    drop(txn);
    Ok(id)
}

/// Every proposal in the document, newest first.
pub fn read_proposals(doc: &yrs::Doc) -> Vec<Value> {
    let map = doc.get_or_insert_map(PROPOSALS_FIELD);
    let txn = doc.transact();
    let mut out: Vec<Value> = map
        .iter(&txn)
        .filter_map(|(_, v)| record_json(&v))
        .collect();
    out.sort_by_key(|p| {
        std::cmp::Reverse(p.get("created_at").and_then(Value::as_i64).unwrap_or(0))
    });
    out
}
