//! A mindmap as a shared document.
//!
//! The nodes used to be rows. They are now one Yjs document per map, for the
//! reason a brainstorm exists at all: two people and an agent talk at the same
//! time, and a store where the last writer wins throws one of them away without
//! saying so.
//!
//! This module is to `/mindmaps` what `docprops.rs` is to `/documents` — pure
//! functions over a `yrs::Doc`, called from inside `room.read` / `room.mutate`
//! so that a write by an agent lands on the same replica the browsers are
//! looking at and shows up without a reload.
//!
//! ## The shape
//!
//! ```text
//! nodes:         Y.Map<node id, Y.Map>
//! relationships: Y.Map<rel id,  Y.Map>
//! ```
//!
//! **A flat map with a parent pointer, not a nested tree.** In a nested CRDT
//! tree a move is delete-plus-insert, which duplicates or loses a subtree when
//! two peers move at once. A parent pointer makes a move one field write that
//! merges cleanly.
//!
//! **But a parent pointer admits cycles**, and that is the price. Two people can
//! each make a perfectly legal move — A under B while B goes under A — and no
//! synchronous validator can see it coming, because neither peer was wrong. So
//! the tree is repaired **on read**, deterministically, by [`normalise`]: every
//! peer computes the same tree from the same state. See its docs for the rules.
//!
//! ## What is a `Y.Text` and what is a scalar
//!
//! `title` and `notes` are `Y.Text`, because two people really do type into the
//! same node and last-write-wins would silently discard one of them. Everything
//! else is a scalar and merges last-write-wins, which is the right answer for a
//! colour or a coordinate.
//!
//! `color`/`shape`/`icons` are separate fields rather than one `style` blob, and
//! that is deliberate: a blob merges as a whole, so two people changing
//! different things about one node clobber each other. A blob also cannot be
//! un-blobbed later without a migration.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

use yrs::types::text::TextPrelim;
use yrs::{
    Any, Array, ArrayPrelim, Doc, GetString, Map, MapPrelim, MapRef, Out, ReadTxn, Text, Transact,
    TransactionMut, XmlFragmentPrelim, XmlFragmentRef,
};

use crate::error::{ApiError, ApiResult};
use crate::fracdex;
use crate::ids::{iso, now_ms};

/// The two roots. Named here so the browser and the server cannot drift.
pub const NODES_FIELD: &str = "nodes";
pub const RELATIONSHIPS_FIELD: &str = "relationships";

/// A node's own prose, as a nested `XmlFragment`.
///
/// The map and the document are two renderings of one thing, so a section's text
/// lives IN its node rather than in a document row copied from it. Tiptap binds
/// to a fragment handed to it directly, which is what makes this possible at
/// all — see `spec/one-model-two-views.md`.
pub const PROSE_KEY: &str = "prose";

/// A node title is a sentence or two, and that brevity is the method rather
/// than a limitation: a branch you cannot read at a glance has stopped being a
/// brainstorm. The long form now has somewhere to go — see [`MAX_NOTES`].
pub const MAX_TITLE: usize = 280;

/// The escape hatch the title cap needs in order to stay honest.
///
/// Notes do not render in the outline; you open a node to read them. So the
/// thing the 280-character rule protects — a branch readable at a glance —
/// survives, and detail no longer has to be truncated or promoted away.
pub const MAX_NOTES: usize = 8_000;

/// A brainstorm, not a database.
pub const MAX_NODES: usize = 500;

/// Past this nobody can read the shape.
pub const MAX_DEPTH: usize = 8;

/// One agent turn.
pub const MAX_GROW: usize = 50;

/// Cross-links are cheap to draw and expensive to read; this is a canvas, not a
/// graph database.
pub const MAX_RELATIONSHIPS: usize = 1_000;

pub const MAX_ATTACHMENTS: usize = 20;
pub const MAX_ICONS: usize = 8;
pub const MAX_EDGE_LABEL: usize = 80;
pub const MAX_REL_LABEL: usize = 80;
pub const MAX_ATTACHMENT_NAME: usize = 200;
pub const MAX_ATTACHMENT_GIST: usize = 500;
pub const MAX_ATTACHMENT_REF: usize = 2_000;

/// What a node can be. `thought` is the default and the overwhelming majority;
/// the rest exist because a map of a system wants to say what a box *is*.
pub const NODE_KINDS: [&str; 5] = ["thought", "question", "decision", "screen", "component"];

/// Who put it there. Stored from the start even though nothing renders it yet,
/// because adding a field to a shared document later means converting
/// everybody's map a second time.
pub const NODE_ORIGINS: [&str; 2] = ["human", "agent"];

pub const ATTACHMENT_KINDS: [&str; 6] = ["pdf", "code", "table", "diagram", "audio", "link"];

// ---------------------------------------------------------------------------
// Plain data, read out of the document
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub gist: String,
    pub reference: String,
}

impl Attachment {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "kind": self.kind,
            "name": self.name,
            "gist": self.gist,
            "ref": self.reference,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Relationship {
    pub id: String,
    pub from: String,
    pub to: String,
    pub label: String,
}

impl Relationship {
    fn to_json(&self) -> Value {
        json!({ "id": self.id, "from": self.from, "to": self.to, "label": self.label })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DocNode {
    pub id: String,
    pub parent: Option<String>,
    pub order: String,
    pub title: String,
    /// The section's prose as plain text — one line per block.
    ///
    /// The prose itself is an `XmlFragment` inside the node, edited by the same
    /// editor `/documents` uses. This is what a CARD shows: a canvas cannot
    /// render ProseMirror and should not try.
    pub notes: String,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub edge_label: String,
    pub kind: String,
    pub origin: String,
    pub reviewed: bool,
    pub icons: Vec<String>,
    pub color: String,
    pub shape: String,
    pub attachments: Vec<Attachment>,
    pub promoted_kind: Option<String>,
    pub promoted_id: Option<String>,
    /// The document this node became, if the map has been turned into one.
    ///
    /// A separate slot from `promoted_*` on purpose: a branch can be an epic AND
    /// appear in the written-up plan, and those are two different facts about
    /// it. Sharing one slot would make converting a map quietly erase what a
    /// branch had graduated into.
    pub document: Option<String>,
    pub created_by: String,
    /// WHICH PERSON made it (`users.id`), where `created_by` is only the
    /// free-form actor string the credential carried.
    ///
    /// The same distinction `cases.human_user` draws, for the same reason: two
    /// `human:alice` tokens are indistinguishable and nothing survives somebody
    /// leaving. Null for a credential bound to nobody — an agent's own token,
    /// or a machine account — which is also how "written by an agent" is read,
    /// rather than by trusting a scope.
    pub created_by_user: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl DocNode {
    /// The wire shape.
    ///
    /// `position` is the **sibling rank**, not the internal order key. The key
    /// is a fractional index — an opaque string that has to be free to change
    /// shape — and publishing it would make somebody depend on it. A rank is
    /// what the old integer `position` meant to a reader, so the contract holds
    /// while the storage underneath does not.
    ///
    /// `at` and `promoted` stay all-or-nothing, exactly as before: half a
    /// coordinate places nothing, and half a promotion names nothing.
    pub fn to_json(&self, mindmap: &str, position: usize) -> Value {
        json!({
            "id": self.id,
            "mindmap": mindmap,
            "parent": self.parent,
            "text": self.title,
            "title": self.title,
            "notes": self.notes,
            "position": position,
            "at": match (self.x, self.y) {
                (Some(x), Some(y)) => json!({ "x": x, "y": y }),
                _ => Value::Null,
            },
            "edge_label": self.edge_label,
            "kind": self.kind,
            "origin": self.origin,
            "reviewed": self.reviewed,
            "icons": self.icons,
            "color": self.color,
            "shape": self.shape,
            "attachments": self.attachments.iter().map(Attachment::to_json).collect::<Vec<_>>(),
            "promoted": match (&self.promoted_kind, &self.promoted_id) {
                (Some(kind), Some(id)) => json!({ "kind": kind, "id": id }),
                _ => Value::Null,
            },
            "document": self.document,
            "created_by": self.created_by,
            "created_by_user": self.created_by_user,
            "created_at": iso(self.created_at),
            "updated_at": iso(self.updated_at),
        })
    }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

fn text_of<T: ReadTxn>(txn: &T, map: &MapRef, key: &str) -> String {
    match map.get(txn, key) {
        // The field as it is meant to be: a shared text a second person can type
        // into without erasing the first.
        Some(Out::YText(t)) => t.get_string(txn),
        // A plain string is accepted on read. A peer may write one, and refusing
        // to show somebody's node because its title arrived in the simpler shape
        // would be a worse answer than showing it.
        Some(Out::Any(Any::String(s))) => s.to_string(),
        _ => String::new(),
    }
}

fn string_of<T: ReadTxn>(txn: &T, map: &MapRef, key: &str) -> Option<String> {
    match map.get(txn, key) {
        Some(Out::Any(Any::String(s))) => Some(s.to_string()),
        Some(Out::YText(t)) => Some(t.get_string(txn)),
        _ => None,
    }
}

fn number_of<T: ReadTxn>(txn: &T, map: &MapRef, key: &str) -> Option<f64> {
    match map.get(txn, key) {
        Some(Out::Any(Any::Number(n))) => Some(n),
        Some(Out::Any(Any::BigInt(n))) => Some(n as f64),
        _ => None,
    }
}

fn int_of<T: ReadTxn>(txn: &T, map: &MapRef, key: &str) -> i64 {
    match map.get(txn, key) {
        Some(Out::Any(Any::BigInt(n))) => n,
        Some(Out::Any(Any::Number(n))) => n as i64,
        _ => 0,
    }
}

fn bool_of<T: ReadTxn>(txn: &T, map: &MapRef, key: &str) -> bool {
    matches!(map.get(txn, key), Some(Out::Any(Any::Bool(true))))
}

fn strings_of<T: ReadTxn>(txn: &T, map: &MapRef, key: &str, cap: usize) -> Vec<String> {
    let Some(Out::YArray(array)) = map.get(txn, key) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for value in array.iter(txn) {
        if let Out::Any(Any::String(s)) = value {
            out.push(s.to_string());
        }
        if out.len() >= cap {
            break;
        }
    }
    out
}

fn attachments_of<T: ReadTxn>(txn: &T, map: &MapRef) -> Vec<Attachment> {
    let Some(Out::YMap(holder)) = map.get(txn, "attachments") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (id, value) in holder.iter(txn) {
        let Out::YMap(entry) = value else { continue };
        out.push(Attachment {
            id: id.to_string(),
            kind: string_of(txn, &entry, "kind").unwrap_or_else(|| "link".to_string()),
            name: string_of(txn, &entry, "name").unwrap_or_default(),
            gist: string_of(txn, &entry, "gist").unwrap_or_default(),
            reference: string_of(txn, &entry, "ref").unwrap_or_default(),
        });
    }
    // A map has no order of its own, so give the reader a stable one.
    out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    out
}

/// Every node in the document, exactly as stored — cycles, orphans and all.
///
/// [`normalise`] is what turns this into a tree. They are separate so the repair
/// rules can be tested without a document.
pub fn read_nodes<T: ReadTxn>(txn: &T, nodes: &MapRef) -> Vec<DocNode> {
    let mut out = Vec::new();
    for (id, value) in nodes.iter(txn) {
        let Out::YMap(entry) = value else { continue };
        out.push(DocNode {
            id: id.to_string(),
            parent: string_of(txn, &entry, "parent"),
            order: string_of(txn, &entry, "order").unwrap_or_default(),
            title: text_of(txn, &entry, "title"),
            // The prose fragment is the truth. A plain `notes` text is what
            // nodes carried before prose existed and is still read, so a map
            // written before the upgrade is legible until `ensure_prose` runs.
            notes: match entry.get(txn, PROSE_KEY) {
                Some(Out::YXmlFragment(frag)) => super::prose::plain_text(txn, &frag),
                _ => text_of(txn, &entry, "notes"),
            },
            x: number_of(txn, &entry, "x"),
            y: number_of(txn, &entry, "y"),
            edge_label: string_of(txn, &entry, "edge_label").unwrap_or_default(),
            kind: string_of(txn, &entry, "kind").unwrap_or_else(|| "thought".to_string()),
            origin: string_of(txn, &entry, "origin").unwrap_or_else(|| "human".to_string()),
            reviewed: bool_of(txn, &entry, "reviewed"),
            icons: strings_of(txn, &entry, "icons", MAX_ICONS),
            color: string_of(txn, &entry, "color").unwrap_or_default(),
            shape: string_of(txn, &entry, "shape").unwrap_or_default(),
            attachments: attachments_of(txn, &entry),
            promoted_kind: string_of(txn, &entry, "promoted_kind"),
            promoted_id: string_of(txn, &entry, "promoted_id"),
            document: string_of(txn, &entry, "document"),
            created_by: string_of(txn, &entry, "created_by").unwrap_or_default(),
            created_by_user: string_of(txn, &entry, "created_by_user"),
            created_at: int_of(txn, &entry, "created_at"),
            updated_at: int_of(txn, &entry, "updated_at"),
        });
    }
    out
}

/// Every cross-link, with the dangling ones already gone.
///
/// A relationship whose end no longer resolves is **dropped rather than
/// repaired**: there is no node to point at, and half an edge is not a fact
/// about anything.
pub fn read_relationships<T: ReadTxn>(
    txn: &T,
    relationships: &MapRef,
    live: &HashSet<String>,
) -> Vec<Relationship> {
    let mut out = Vec::new();
    for (id, value) in relationships.iter(txn) {
        let Out::YMap(entry) = value else { continue };
        let (Some(from), Some(to)) = (string_of(txn, &entry, "from"), string_of(txn, &entry, "to"))
        else {
            continue;
        };
        if !live.contains(&from) || !live.contains(&to) {
            continue;
        }
        out.push(Relationship {
            id: id.to_string(),
            from,
            to,
            label: string_of(txn, &entry, "label").unwrap_or_default(),
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

// ---------------------------------------------------------------------------
// Normalisation — the price of a parent pointer, paid deterministically
// ---------------------------------------------------------------------------

/// Repair the parent pointers into an actual tree.
///
/// Two rules, and both are chosen so that every peer reaches the same answer
/// from the same state — which is the only property that matters here. A repair
/// that depended on iteration order would have two people looking at two
/// different maps and no way to notice.
///
/// 1. **A cycle.** Two peers can each make a legal move that together form a
///    loop; neither was wrong, and no synchronous check could have caught it.
///    The member with the **lowest id** is re-parented to the root, so the
///    choice is a property of the data rather than of who read it first.
/// 2. **An orphan** — a node whose parent was deleted concurrently — is
///    re-parented to the root, never dropped. Losing a subtree because somebody
///    else pruned its parent is a far worse outcome than a branch appearing at
///    the top level where it can be seen and moved.
///
/// Depth is deliberately **not** enforced here. Creating past the cap is
/// refused at the API, but silently moving somebody's node because a branch got
/// deep would be a repair nobody asked for.
pub fn normalise(mut nodes: Vec<DocNode>) -> Vec<DocNode> {
    let live: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();

    // Orphans first: a node pointing at nothing is not in a cycle, and clearing
    // it here keeps the walk below simple.
    for node in &mut nodes {
        if let Some(parent) = &node.parent {
            if !live.contains(parent) {
                node.parent = None;
            }
        }
    }

    let parents: HashMap<String, Option<String>> = nodes
        .iter()
        .map(|n| (n.id.clone(), n.parent.clone()))
        .collect();

    // Every node that sits on or under a cycle.
    let mut cut: HashSet<String> = HashSet::new();
    for node in &nodes {
        let mut seen: Vec<String> = Vec::new();
        let mut cursor = Some(node.id.clone());
        while let Some(id) = cursor {
            if seen.contains(&id) {
                // `id` is the entry point of the loop; collect the loop itself,
                // which is the tail of `seen` from that point on.
                let at = seen.iter().position(|s| *s == id).unwrap_or(0);
                let loop_members: Vec<String> = seen[at..].to_vec();
                if let Some(lowest) = loop_members.iter().min() {
                    cut.insert(lowest.clone());
                }
                break;
            }
            seen.push(id.clone());
            cursor = parents.get(&id).cloned().flatten();
        }
    }

    for node in &mut nodes {
        if cut.contains(&node.id) {
            node.parent = None;
        }
    }
    nodes
}

/// Depth-first, parents before children, siblings in order.
///
/// This is the order `nodes[]` comes back in. It replaces a flat global sort by
/// `position`, which stopped meaning anything once order became per-ring — and
/// pre-order is what every reader of this array actually rebuilds: the CLI, the
/// canvas and an agent reading an outline all want a parent before its children.
///
/// Siblings sort by the fractional order key with the **node id as tiebreak**,
/// so two peers that independently minted the same key still agree. A key that
/// is not one this server would have produced sorts last rather than anywhere
/// unpredictable.
pub fn tree_order(nodes: &[DocNode]) -> Vec<&DocNode> {
    let mut children: HashMap<Option<String>, Vec<&DocNode>> = HashMap::new();
    for node in nodes {
        children.entry(node.parent.clone()).or_default().push(node);
    }
    for ring in children.values_mut() {
        ring.sort_by(|a, b| {
            let a_ok = fracdex::is_valid(&a.order);
            let b_ok = fracdex::is_valid(&b.order);
            // When BOTH keys are unusable the key is ignored entirely and the id
            // decides. Comparing junk against junk would be an ordering the
            // browser does not reproduce — and two peers drawing the same map in
            // two different orders is precisely what this whole comparator is
            // here to prevent. `web/src/lib/mindmap-doc.ts` has the same three
            // steps in the same order.
            b_ok.cmp(&a_ok)
                .then_with(|| {
                    if a_ok && b_ok {
                        a.order.cmp(&b.order)
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
                .then_with(|| a.id.cmp(&b.id))
        });
    }

    let mut out: Vec<&DocNode> = Vec::with_capacity(nodes.len());
    let mut stack: Vec<&DocNode> = children
        .get(&None)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .rev()
        .collect();
    while let Some(node) = stack.pop() {
        out.push(node);
        if let Some(ring) = children.get(&Some(node.id.clone())) {
            for child in ring.iter().rev() {
                stack.push(child);
            }
        }
    }
    // Every node has exactly one parent, so the walk visits each at most once and
    // cannot loop. It CAN however return fewer nodes than it was given: anything
    // still sitting on a cycle is unreachable from the root ring. That is why
    // callers pass `normalise`d input, and why this asserts rather than trusting
    // it — silently returning a partial map would break the rule that a list
    // says what it left out.
    debug_assert_eq!(
        out.len(),
        nodes.len(),
        "tree_order lost nodes — its input was not normalised"
    );
    out
}

/// Each node's rank among its own siblings, which is what `position` means on
/// the wire.
pub fn ranks(nodes: &[DocNode]) -> HashMap<String, usize> {
    let ordered = tree_order(nodes);
    let mut seen: HashMap<Option<String>, usize> = HashMap::new();
    let mut out = HashMap::new();
    for node in ordered {
        let slot = seen.entry(node.parent.clone()).or_insert(0);
        out.insert(node.id.clone(), *slot);
        *slot += 1;
    }
    out
}

/// How deep a node sits, counting the first ring as 1.
pub fn depth_of(nodes: &[DocNode], id: &str) -> usize {
    let parents: HashMap<&str, Option<&str>> = nodes
        .iter()
        .map(|n| (n.id.as_str(), n.parent.as_deref()))
        .collect();
    let mut depth = 0usize;
    let mut cursor = Some(id);
    while let Some(current) = cursor {
        depth += 1;
        if depth > MAX_DEPTH + 1 {
            break;
        }
        cursor = parents.get(current).copied().flatten();
    }
    depth
}

/// A node and everything under it, parents first.
pub fn subtree<'a>(nodes: &'a [DocNode], root: &str) -> Vec<&'a DocNode> {
    let ordered = tree_order(nodes);
    let mut wanted: HashSet<String> = HashSet::new();
    wanted.insert(root.to_string());
    let mut out = Vec::new();
    for node in ordered {
        let included = node.id == root
            || node
                .parent
                .as_ref()
                .is_some_and(|parent| wanted.contains(parent));
        if included {
            wanted.insert(node.id.clone());
            out.push(node);
        }
    }
    out
}

/// The indented text an agent reads, and the body a promotion carries.
///
/// Two spaces per level, `- ` per node — the shape asserted by the tests that
/// existed before any of this was a CRDT, and the cheapest shape for a model to
/// reason about.
pub fn outline(nodes: &[DocNode], root: &str) -> String {
    let branch = subtree(nodes, root);
    let mut depth: HashMap<&str, usize> = HashMap::new();
    let mut out = String::new();
    for node in branch {
        let level = node
            .parent
            .as_deref()
            .and_then(|parent| depth.get(parent).copied().map(|d| d + 1))
            .unwrap_or(0);
        depth.insert(node.id.as_str(), level);
        out.push_str(&"  ".repeat(level));
        out.push_str("- ");
        out.push_str(&node.title);
        out.push('\n');
    }
    out
}

/// The whole map as indented text, under its title.
pub fn full_outline(nodes: &[DocNode], title: &str) -> String {
    let mut out = format!("# {title}\n");
    for node in tree_order(nodes) {
        if node.parent.is_none() {
            out.push_str(&outline(nodes, &node.id));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Turning a map into documents
// ---------------------------------------------------------------------------

/// One document a conversion would make, or refresh.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentPlan {
    /// The node it comes from.
    pub node: String,
    pub title: String,
    /// Folder, `/`-separated — the ancestry that led here.
    pub path: String,
    /// What its prose opens with. Empty for a node that is only a heading.
    pub blocks: Vec<super::prose::Block>,
    /// The document this node already became, if the map has been converted
    /// before.
    pub existing: Option<String>,
}

/// A path segment cannot be empty, `.`, `..`, or carry a `/`.
///
/// A node's title is free text, so it has to be made safe for a folder name
/// rather than trusted as one. The result is still recognisably the title —
/// this is not slugification, because a person is going to read this path.
fn path_segment(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if c == '/' || c == '\\' { '-' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    if cleaned.is_empty() {
        "untitled".to_string()
    } else {
        cleaned.chars().take(60).collect()
    }
}

/// Which nodes become documents, where they sit, and what they open with.
///
/// **The rule that makes a converted map readable**: a node becomes a document
/// when it has children or something written on it. A bare leaf — a phrase with
/// no notes and nothing under it — becomes a BULLET in its parent's document
/// instead. Without that rule every six-word thought turns into its own page,
/// and a map of forty nodes converts into forty documents nobody will open.
///
/// Ancestry becomes the folder path, so the shape of the thinking survives as
/// the shape of the folder tree. `/documents` already builds its tree from
/// paths, which is why a map does not need a section model to convert into
/// something navigable.
pub fn plan_documents(nodes: &[DocNode], root_folder: &str) -> Vec<DocumentPlan> {
    let ordered = tree_order(nodes);
    let children: HashMap<&str, Vec<&DocNode>> = {
        let mut map: HashMap<&str, Vec<&DocNode>> = HashMap::new();
        for node in &ordered {
            if let Some(parent) = node.parent.as_deref() {
                map.entry(parent).or_default().push(node);
            }
        }
        map
    };

    let becomes_document = |n: &DocNode| -> bool {
        !n.notes.trim().is_empty() || children.contains_key(n.id.as_str())
    };

    // Where each document-node's own folder is, so its children can be filed
    // underneath it.
    let mut folder_of: HashMap<&str, String> = HashMap::new();
    let mut out = Vec::new();

    for node in &ordered {
        if !becomes_document(node) {
            continue;
        }
        let parent_folder = node
            .parent
            .as_deref()
            .and_then(|p| folder_of.get(p).cloned())
            .unwrap_or_else(|| root_folder.to_string());

        let mut blocks = Vec::new();
        if !node.notes.trim().is_empty() {
            blocks.push(super::prose::Block::Paragraph(
                node.notes.trim().to_string(),
            ));
        }
        let bullets: Vec<String> = children
            .get(node.id.as_str())
            .map(|kids| {
                kids.iter()
                    .filter(|k| !becomes_document(k))
                    .map(|k| k.title.clone())
                    .collect()
            })
            .unwrap_or_default();
        if !bullets.is_empty() {
            blocks.push(super::prose::Block::Bullets(bullets));
        }

        let own_folder = if parent_folder.is_empty() {
            path_segment(&node.title)
        } else {
            format!("{parent_folder}/{}", path_segment(&node.title))
        };
        folder_of.insert(node.id.as_str(), own_folder);

        out.push(DocumentPlan {
            node: node.id.clone(),
            title: node.title.clone(),
            path: parent_folder,
            blocks,
            existing: node.document.clone(),
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Get the two roots, creating them if this is a fresh map.
pub fn roots(doc: &Doc) -> (MapRef, MapRef) {
    (
        doc.get_or_insert_map(NODES_FIELD),
        doc.get_or_insert_map(RELATIONSHIPS_FIELD),
    )
}

/// What a caller asks for when adding a node.
#[derive(Debug, Clone, Default)]
pub struct NodeAdd {
    pub parent: Option<String>,
    /// The person behind the credential, when it is bound to one.
    pub by_user: Option<String>,
    pub title: String,
    pub notes: Option<String>,
    /// Where among its siblings, as an index. `None` appends.
    pub position: Option<usize>,
    pub kind: Option<String>,
    pub origin: Option<String>,
    pub edge_label: Option<String>,
}

/// What a caller may change about one node. `Some(None)` and `None` mean
/// different things throughout: absent leaves the field alone, explicit null
/// clears it.
#[derive(Debug, Clone, Default)]
pub struct NodePatch {
    pub title: Option<String>,
    pub notes: Option<String>,
    pub parent: Option<Option<String>>,
    pub position: Option<usize>,
    pub at: Option<Option<(f64, f64)>>,
    pub kind: Option<String>,
    pub edge_label: Option<String>,
    pub color: Option<String>,
    pub shape: Option<String>,
    pub icons: Option<Vec<String>>,
    pub reviewed: Option<bool>,
}

pub fn validate_title(title: &str) -> ApiResult<String> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(ApiError::validation(
            "validation.mindmap_node_text",
            "A node needs some text — an empty thought is not one.",
        ));
    }
    let count = trimmed.chars().count();
    if count > MAX_TITLE {
        return Err(ApiError::validation(
            "validation.mindmap_node_text",
            format!(
                "That title is {count} characters and the cap is {MAX_TITLE}. A mindmap node is a sentence or two — that brevity is what makes a branch readable at a glance."
            ),
        )
        .remedy(
            "Shorten it, move the detail into the node's notes, split it into two nodes, or promote the branch to an initiative (POST /v1/mindmaps/{id}/nodes/{node}/promote) where the long form belongs."
                .to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

pub fn validate_notes(notes: &str) -> ApiResult<String> {
    let count = notes.chars().count();
    if count > MAX_NOTES {
        return Err(ApiError::validation(
            "validation.mindmap_notes",
            format!(
                "Those notes are {count} characters and the cap is {MAX_NOTES}. Notes are the long form of one thought, not a document — at this length the branch wants to be an initiative."
            ),
        ));
    }
    Ok(notes.to_string())
}

/// The message for a value outside a small allowed set, or `None` if it is fine.
///
/// Returns the MESSAGE rather than the error, so every caller constructs its own
/// `ApiError` with a literal code. That is not ceremony: the error-code guard in
/// `tests/api.rs` reads the source text, and a code passed in as an argument is
/// invisible to it — which would quietly drop this file out of the check that
/// keeps `x-error-codes` honest.
fn not_one_of(value: &str, allowed: &[&str], noun: &str) -> Option<String> {
    if allowed.contains(&value) {
        return None;
    }
    Some(format!(
        "Unknown {noun} '{value}'. Use one of: {}.",
        allowed.join(", ")
    ))
}

pub fn validate_kind(kind: &str) -> ApiResult<()> {
    match not_one_of(kind, &NODE_KINDS, "node kind") {
        Some(message) => Err(ApiError::validation(
            "validation.mindmap_node_kind",
            message,
        )),
        None => Ok(()),
    }
}

pub fn validate_origin(origin: &str) -> ApiResult<()> {
    match not_one_of(origin, &NODE_ORIGINS, "node origin") {
        Some(message) => Err(ApiError::validation(
            "validation.mindmap_node_origin",
            message,
        )),
        None => Ok(()),
    }
}

/// The trimmed value, plus the message if it is over the cap.
///
/// Same shape and same reason as [`not_one_of`]: the code stays a literal at the
/// call site so the error-code guard can read it.
fn cap_check(value: &str, cap: usize, noun: &str) -> (String, Option<String>) {
    let trimmed = value.trim().to_string();
    let count = trimmed.chars().count();
    let message =
        (count > cap).then(|| format!("That {noun} is {count} characters and the cap is {cap}."));
    (trimmed, message)
}

/// The order key for a new sibling at `position`, or appended when it is `None`.
fn order_for(siblings: &[&DocNode], position: Option<usize>) -> String {
    let keys: Vec<&str> = siblings.iter().map(|n| n.order.as_str()).collect();
    match position {
        None => fracdex::between(keys.last().copied(), None),
        Some(at) => {
            // Clamped, not trusted. `check_position` refuses an out-of-range
            // index at the API, but an internal caller that got here with one
            // would find BOTH neighbours missing — and `between(None, None)`
            // returns the FIRST key in the ring, so "put it past the end" would
            // quietly mean "put it at the front".
            let at = at.min(keys.len());
            let before = if at == 0 {
                None
            } else {
                keys.get(at - 1).copied()
            };
            let after = keys.get(at).copied();
            fracdex::between(before, after)
        }
    }
}

/// `position` used to be a gapped integer the caller chose (1000, 2000, …). It
/// is an index among siblings now, so an old value like 1500 sits past the end
/// of every ring — and silently appending would answer a misplaced node with a
/// 201. Refuse it, and say what changed.
fn check_position(position: Option<usize>, ring: usize) -> ApiResult<()> {
    match position {
        Some(p) if p > ring => Err(ApiError::validation(
            "validation.mindmap_position",
            format!(
                "Field 'position' is {p}, but that ring holds {ring} node(s), so the largest position is {ring}. It is an INDEX among siblings now, not the gapped 1000/2000 ordering it used to be — pass 0 for first, or leave it out to append."
            ),
        )),
        _ => Ok(()),
    }
}

fn siblings_of<'a>(nodes: &'a [DocNode], parent: Option<&str>) -> Vec<&'a DocNode> {
    let ordered = tree_order(nodes);
    ordered
        .into_iter()
        .filter(|n| n.parent.as_deref() == parent)
        .collect()
}

/// Write one node into the document.
#[allow(clippy::too_many_arguments)]
fn put_node(
    txn: &mut TransactionMut,
    nodes: &MapRef,
    id: &str,
    add: &NodeAdd,
    order: &str,
    actor: &str,
    now: i64,
) {
    let entry = nodes.insert(txn, id.to_string(), MapPrelim::default());
    match &add.parent {
        Some(parent) => entry.insert(txn, "parent", parent.clone()),
        // Written as an explicit null rather than left absent, so "off the root"
        // is a fact in the document instead of the absence of one.
        None => entry.insert(txn, "parent", Any::Null),
    };
    entry.insert(txn, "order", order.to_string());
    entry.insert(txn, "title", TextPrelim::new(add.title.clone()));
    // The section's prose, empty until somebody writes in it. A fragment rather
    // than a text, because this is what the document view's editor binds to.
    let prose: XmlFragmentRef = entry.insert(txn, PROSE_KEY, XmlFragmentPrelim::default());
    if let Some(notes) = add.notes.as_deref() {
        if !notes.trim().is_empty() {
            super::prose::set_plain_text(txn, &prose, notes);
        }
    }
    entry.insert(txn, "x", Any::Null);
    entry.insert(txn, "y", Any::Null);
    entry.insert(
        txn,
        "edge_label",
        add.edge_label.clone().unwrap_or_default(),
    );
    entry.insert(
        txn,
        "kind",
        add.kind.clone().unwrap_or_else(|| "thought".to_string()),
    );
    entry.insert(
        txn,
        "origin",
        add.origin.clone().unwrap_or_else(|| "human".to_string()),
    );
    entry.insert(txn, "reviewed", false);
    entry.insert(txn, "icons", ArrayPrelim::from(Vec::<String>::new()));
    entry.insert(txn, "color", String::new());
    entry.insert(txn, "shape", String::new());
    entry.insert(txn, "attachments", MapPrelim::default());
    entry.insert(txn, "promoted_kind", Any::Null);
    entry.insert(txn, "promoted_id", Any::Null);
    entry.insert(txn, "document", Any::Null);
    entry.insert(txn, "created_by", actor.to_string());
    match &add.by_user {
        Some(user) => entry.insert(txn, "created_by_user", user.clone()),
        None => entry.insert(txn, "created_by_user", Any::Null),
    };
    entry.insert(txn, "created_at", now);
    entry.insert(txn, "updated_at", now);
}

/// Add a batch of nodes, whole or not at all.
///
/// A batch, because that is what an agent adding a branch sends: half a branch
/// arriving would leave a map nobody asked for and no way to tell which half.
pub fn add_nodes(doc: &Doc, adds: &[NodeAdd], actor: &str) -> ApiResult<Vec<(String, NodeAdd)>> {
    if adds.is_empty() {
        return Err(ApiError::validation(
            "validation.mindmap_nodes",
            "Send at least one node to add.",
        ));
    }
    if adds.len() > MAX_GROW {
        return Err(ApiError::validation(
            "validation.mindmap_nodes",
            format!(
                "{} nodes is over the cap of {MAX_GROW} per call. Each one costs statements inside the transaction that serializes every claim and transition in the store, so a batch has to be bounded.",
                adds.len()
            ),
        ));
    }

    let (nodes_map, _) = roots(doc);
    let mut txn = doc.transact_mut();
    let existing = normalise(read_nodes(&txn, &nodes_map));

    if existing.len() + adds.len() > MAX_NODES {
        return Err(ApiError::conflict(
            "mindmap.full",
            format!(
                "This map holds {} nodes and the cap is {MAX_NODES}. A brainstorm this big has stopped being one — promote its branches into initiatives or epics, or start a second map.",
                existing.len()
            ),
        ));
    }

    // Validate every one before writing any: the batch lands whole or not at all.
    let mut prepared: Vec<(String, NodeAdd)> = Vec::with_capacity(adds.len());
    let live: HashSet<&str> = existing.iter().map(|n| n.id.as_str()).collect();
    // A batch may name a parent it created a moment ago, so the depth of a node
    // in this batch is not knowable from `existing` — `depth_of` would report 1
    // for an id it has never seen and wave a fifty-deep chain straight through
    // the cap. This carries the running depth of everything the batch adds.
    let mut batch_depth: HashMap<String, usize> = HashMap::new();
    for add in adds {
        let title = validate_title(&add.title)?;
        let notes = validate_notes(add.notes.as_deref().unwrap_or_default())?;
        if let Some(kind) = &add.kind {
            validate_kind(kind)?;
        }
        if let Some(origin) = &add.origin {
            validate_origin(origin)?;
        }
        let edge_label = {
            let (value, over) = cap_check(
                add.edge_label.as_deref().unwrap_or_default(),
                MAX_EDGE_LABEL,
                "edge label",
            );
            if let Some(message) = over {
                return Err(ApiError::validation(
                    "validation.mindmap_edge_label",
                    message,
                ));
            }
            value
        };
        let depth = match &add.parent {
            None => 1,
            Some(parent) => {
                let known_here = batch_depth.get(parent.as_str()).copied();
                if !live.contains(parent.as_str()) && known_here.is_none() {
                    return Err(ApiError::not_found("mindmap_node", parent));
                }
                let parent_depth = known_here.unwrap_or_else(|| depth_of(&existing, parent));
                if parent_depth >= MAX_DEPTH {
                    return Err(ApiError::conflict(
                        "mindmap.too_deep",
                        format!(
                            "That would nest {} levels deep and the cap is {MAX_DEPTH}. Past this nobody can read the shape — promote the branch instead.",
                            parent_depth + 1
                        ),
                    ));
                }
                parent_depth + 1
            }
        };
        check_position(
            add.position,
            siblings_of(&existing, add.parent.as_deref()).len(),
        )?;
        let id = crate::ids::mindmap_node_id();
        batch_depth.insert(id.clone(), depth);
        prepared.push((
            id,
            NodeAdd {
                parent: add.parent.clone(),
                by_user: add.by_user.clone(),
                title,
                notes: Some(notes),
                position: add.position,
                kind: add.kind.clone(),
                origin: add.origin.clone(),
                edge_label: Some(edge_label),
            },
        ));
    }

    let now = now_ms();
    // The ring is recomputed per insert so two nodes added to the same ring in
    // one batch land in the order they were sent rather than on top of each
    // other.
    let mut working = existing;
    for (id, add) in &prepared {
        let ring = siblings_of(&working, add.parent.as_deref());
        let order = order_for(&ring, add.position);
        put_node(&mut txn, &nodes_map, id, add, &order, actor, now);
        working.push(DocNode {
            id: id.clone(),
            created_by_user: add.by_user.clone(),
            parent: add.parent.clone(),
            order,
            title: add.title.clone(),
            notes: add.notes.clone().unwrap_or_default(),
            x: None,
            y: None,
            edge_label: add.edge_label.clone().unwrap_or_default(),
            kind: add.kind.clone().unwrap_or_else(|| "thought".to_string()),
            origin: add.origin.clone().unwrap_or_else(|| "human".to_string()),
            reviewed: false,
            icons: Vec::new(),
            color: String::new(),
            shape: String::new(),
            attachments: Vec::new(),
            promoted_kind: None,
            promoted_id: None,
            document: None,
            created_by: actor.to_string(),
            created_at: now,
            updated_at: now,
        });
    }

    Ok(prepared)
}

/// A node's prose fragment, made if this node predates prose.
fn prose_of(txn: &mut TransactionMut, entry: &MapRef) -> XmlFragmentRef {
    match entry.get(txn, PROSE_KEY) {
        Some(Out::YXmlFragment(frag)) => frag,
        _ => entry.insert(txn, PROSE_KEY, XmlFragmentPrelim::default()),
    }
}

/// Move a map written before prose existed into it.
///
/// Nodes used to carry `notes` as a plain `Y.Text`. The document view edits a
/// fragment, so every node needs one — and leaving both would be exactly the
/// two-places-for-one-paragraph the whole design removes. Returns whether it
/// changed anything, so an untouched map broadcasts nothing.
///
/// Idempotent, and safe to run on every room open: a node that already has
/// prose is skipped, and a node whose legacy notes are empty is left alone.
pub fn ensure_prose(doc: &Doc) -> bool {
    let (nodes_map, _) = roots(doc);
    let mut txn = doc.transact_mut();
    let ids: Vec<String> = nodes_map.iter(&txn).map(|(id, _)| id.to_string()).collect();
    let mut changed = false;
    for id in ids {
        let Some(Out::YMap(entry)) = nodes_map.get(&txn, &id) else {
            continue;
        };
        if matches!(entry.get(&txn, PROSE_KEY), Some(Out::YXmlFragment(_))) {
            continue;
        }
        let legacy = text_of(&txn, &entry, "notes");
        let prose = entry.insert(&mut txn, PROSE_KEY, XmlFragmentPrelim::default());
        if !legacy.trim().is_empty() {
            super::prose::set_plain_text(&mut txn, &prose, &legacy);
        }
        // The old field goes, so nobody can write to a place nothing reads.
        entry.remove(&mut txn, "notes");
        changed = true;
    }
    changed
}

fn node_map<T: ReadTxn>(txn: &T, nodes: &MapRef, id: &str) -> ApiResult<MapRef> {
    match nodes.get(txn, id) {
        Some(Out::YMap(entry)) => Ok(entry),
        _ => Err(ApiError::not_found("mindmap_node", id)),
    }
}

/// Replace a shared text's whole content.
///
/// A wholesale replace rather than a diff, because this is the API path: a
/// caller sent the finished string. Somebody typing in a browser edits the same
/// `Y.Text` character by character over the socket, which is where the merge
/// actually matters.
fn set_text(txn: &mut TransactionMut, entry: &MapRef, key: &str, value: &str) {
    let text = match entry.get(txn, key) {
        Some(Out::YText(existing)) => existing,
        _ => entry.insert(txn, key, TextPrelim::new("")),
    };
    let len = text.len(txn);
    if len > 0 {
        text.remove_range(txn, 0, len);
    }
    if !value.is_empty() {
        text.insert(txn, 0, value);
    }
}

/// Change one node.
pub fn patch_node(doc: &Doc, id: &str, patch: &NodePatch, _actor: &str) -> ApiResult<()> {
    let (nodes_map, _) = roots(doc);
    let mut txn = doc.transact_mut();
    let all = normalise(read_nodes(&txn, &nodes_map));
    if !all.iter().any(|n| n.id == id) {
        return Err(ApiError::not_found("mindmap_node", id));
    }

    // Validate everything before touching the document, so a refused patch
    // leaves the node exactly as it was.
    let title = patch.title.as_deref().map(validate_title).transpose()?;
    let notes = patch.notes.as_deref().map(validate_notes).transpose()?;
    if let Some(kind) = &patch.kind {
        validate_kind(kind)?;
    }
    let edge_label = match patch.edge_label.as_deref() {
        Some(value) => {
            let (value, over) = cap_check(value, MAX_EDGE_LABEL, "edge label");
            if let Some(message) = over {
                return Err(ApiError::validation(
                    "validation.mindmap_edge_label",
                    message,
                ));
            }
            Some(value)
        }
        None => None,
    };
    {
        let ring_parent = match &patch.parent {
            Some(target) => target.as_deref(),
            None => all
                .iter()
                .find(|n| n.id == id)
                .and_then(|n| n.parent.as_deref()),
        };
        let ring = siblings_of(&all, ring_parent)
            .into_iter()
            .filter(|n| n.id != id)
            .count();
        check_position(patch.position, ring)?;
    }
    if let Some(icons) = &patch.icons {
        if icons.len() > MAX_ICONS {
            return Err(ApiError::validation(
                "validation.mindmap_icons",
                format!("A node carries at most {MAX_ICONS} icons."),
            ));
        }
    }

    // A reparent is the one change that can break the tree, so it is checked
    // against the tree rather than against the node.
    if let Some(Some(parent)) = &patch.parent {
        {
            if parent == id {
                return Err(ApiError::conflict(
                    "mindmap.cycle",
                    "A node cannot hang off itself.",
                ));
            }
            if !all.iter().any(|n| n.id == *parent) {
                return Err(ApiError::not_found("mindmap_node", parent));
            }
            if subtree(&all, id).iter().any(|n| n.id == *parent) {
                return Err(ApiError::conflict(
                    "mindmap.cycle",
                    "That would hang a node off one of its own children, which would cut the branch off the map.",
                ));
            }
            // The destination's depth is only half of it: dragging a branch
            // takes everything under it along, so a shallow-looking move can
            // still bury a subtree past the cap.
            let landing = depth_of(&all, parent) + 1;
            let moved_height = subtree(&all, id)
                .iter()
                .map(|n| depth_of(&all, &n.id))
                .max()
                .unwrap_or(1)
                - depth_of(&all, id);
            if landing + moved_height > MAX_DEPTH {
                return Err(ApiError::conflict(
                    "mindmap.too_deep",
                    format!(
                        "That would nest {} levels deep and the cap is {MAX_DEPTH}. The branch being moved is {} levels tall, and it all comes along.",
                        landing + moved_height,
                        moved_height + 1
                    ),
                ));
            }
        }
    }

    let entry = node_map(&txn, &nodes_map, id)?;
    let now = now_ms();

    if let Some(target) = &patch.parent {
        match target {
            Some(parent) => entry.insert(&mut txn, "parent", parent.clone()),
            None => entry.insert(&mut txn, "parent", Any::Null),
        };
        // A move lands at the end of its new ring unless the caller said where,
        // which is what "drag it over there" means.
        let ring: Vec<&DocNode> = siblings_of(&all, target.as_deref())
            .into_iter()
            .filter(|n| n.id != id)
            .collect();
        let order = order_for(&ring, patch.position);
        entry.insert(&mut txn, "order", order);
    } else if let Some(position) = patch.position {
        let current = all.iter().find(|n| n.id == id).expect("node present");
        let ring: Vec<&DocNode> = siblings_of(&all, current.parent.as_deref())
            .into_iter()
            .filter(|n| n.id != id)
            .collect();
        entry.insert(&mut txn, "order", order_for(&ring, Some(position)));
    }

    if let Some(title) = title {
        set_text(&mut txn, &entry, "title", &title);
    }
    if let Some(notes) = notes {
        // `notes` on the wire is the section's prose as plain text. A caller
        // that sends a finished string replaces the prose with it; somebody
        // typing in the document view edits the same fragment through the
        // editor, which is where the merge actually matters.
        let prose = prose_of(&mut txn, &entry);
        super::prose::set_plain_text(&mut txn, &prose, &notes);
    }
    if let Some(at) = &patch.at {
        match at {
            Some((x, y)) => {
                entry.insert(&mut txn, "x", *x);
                entry.insert(&mut txn, "y", *y);
            }
            // Both cleared together. Half a coordinate places nothing.
            None => {
                entry.insert(&mut txn, "x", Any::Null);
                entry.insert(&mut txn, "y", Any::Null);
            }
        }
    }
    if let Some(kind) = &patch.kind {
        entry.insert(&mut txn, "kind", kind.clone());
    }
    if let Some(label) = edge_label {
        entry.insert(&mut txn, "edge_label", label);
    }
    if let Some(color) = &patch.color {
        entry.insert(&mut txn, "color", color.clone());
    }
    if let Some(shape) = &patch.shape {
        entry.insert(&mut txn, "shape", shape.clone());
    }
    if let Some(icons) = &patch.icons {
        entry.insert(&mut txn, "icons", ArrayPrelim::from(icons.clone()));
    }
    if let Some(reviewed) = patch.reviewed {
        entry.insert(&mut txn, "reviewed", reviewed);
    }
    entry.insert(&mut txn, "updated_at", now);
    Ok(())
}

/// Prune a node and everything under it, and take its cross-links with it.
///
/// Returns how many nodes went. The subtree goes because that is what pruning a
/// branch means; the relationships go because an edge to a node that no longer
/// exists is not a fact about anything.
pub fn delete_node(doc: &Doc, id: &str) -> ApiResult<usize> {
    let (nodes_map, rels_map) = roots(doc);
    let mut txn = doc.transact_mut();
    let all = normalise(read_nodes(&txn, &nodes_map));
    if !all.iter().any(|n| n.id == id) {
        return Err(ApiError::not_found("mindmap_node", id));
    }

    let doomed: Vec<String> = subtree(&all, id).iter().map(|n| n.id.clone()).collect();
    let gone: HashSet<&str> = doomed.iter().map(String::as_str).collect();

    let dangling: Vec<String> = rels_map
        .iter(&txn)
        .filter_map(|(rel_id, value)| {
            let Out::YMap(entry) = value else { return None };
            let from = string_of(&txn, &entry, "from")?;
            let to = string_of(&txn, &entry, "to")?;
            (gone.contains(from.as_str()) || gone.contains(to.as_str())).then(|| rel_id.to_string())
        })
        .collect();

    for rel_id in dangling {
        rels_map.remove(&mut txn, &rel_id);
    }
    for node_id in &doomed {
        nodes_map.remove(&mut txn, node_id);
    }
    Ok(doomed.len())
}

/// Record which document a node became.
pub fn set_document(doc: &Doc, id: &str, document: &str) -> ApiResult<()> {
    let (nodes_map, _) = roots(doc);
    let mut txn = doc.transact_mut();
    let entry = node_map(&txn, &nodes_map, id)?;
    entry.insert(&mut txn, "document", document.to_string());
    entry.insert(&mut txn, "updated_at", now_ms());
    Ok(())
}

/// Record what a branch became.
///
/// Called after the epic or initiative exists, never before: a link written
/// first would point at nothing if the write behind it failed.
pub fn set_promoted(doc: &Doc, id: &str, kind: &str, target: &str) -> ApiResult<()> {
    let (nodes_map, _) = roots(doc);
    let mut txn = doc.transact_mut();
    let entry = node_map(&txn, &nodes_map, id)?;
    entry.insert(&mut txn, "promoted_kind", kind.to_string());
    entry.insert(&mut txn, "promoted_id", target.to_string());
    entry.insert(&mut txn, "updated_at", now_ms());
    Ok(())
}

// ---------------------------------------------------------------------------
// Relationships — an edge that is not part of the hierarchy
// ---------------------------------------------------------------------------

/// Link two nodes without making one the parent of the other.
///
/// The hierarchy answers "what is this part of". This answers everything else —
/// a question hanging off the thing it questions, a screen that navigates to
/// another, a "see also". One mechanism instead of three special cases, which is
/// why it is a separate collection rather than more fields on a node.
pub fn add_relationship(doc: &Doc, from: &str, to: &str, label: &str) -> ApiResult<Relationship> {
    let label = {
        let (value, over) = cap_check(label, MAX_REL_LABEL, "label");
        if let Some(message) = over {
            return Err(ApiError::validation(
                "validation.mindmap_rel_label",
                message,
            ));
        }
        value
    };
    let (nodes_map, rels_map) = roots(doc);
    let mut txn = doc.transact_mut();

    if from == to {
        return Err(ApiError::validation(
            "validation.mindmap_relationship",
            "A relationship needs two different nodes; an edge from a node to itself says nothing.",
        ));
    }
    for end in [from, to] {
        if !matches!(nodes_map.get(&txn, end), Some(Out::YMap(_))) {
            return Err(ApiError::not_found("mindmap_node", end));
        }
    }
    if rels_map.len(&txn) as usize >= MAX_RELATIONSHIPS {
        return Err(ApiError::conflict(
            "mindmap.relationships_full",
            format!(
                "This map already holds {MAX_RELATIONSHIPS} relationships. Past that the canvas is a graph nobody can read — split the map."
            ),
        ));
    }

    let id = crate::ids::mindmap_relationship_id();
    let entry = rels_map.insert(&mut txn, id.clone(), MapPrelim::default());
    entry.insert(&mut txn, "from", from.to_string());
    entry.insert(&mut txn, "to", to.to_string());
    entry.insert(&mut txn, "label", label.clone());
    entry.insert(&mut txn, "created_at", now_ms());

    Ok(Relationship {
        id,
        from: from.to_string(),
        to: to.to_string(),
        label,
    })
}

pub fn delete_relationship(doc: &Doc, id: &str) -> ApiResult<()> {
    let (_, rels_map) = roots(doc);
    let mut txn = doc.transact_mut();
    if !matches!(rels_map.get(&txn, id), Some(Out::YMap(_))) {
        return Err(ApiError::not_found("mindmap_relationship", id));
    }
    rels_map.remove(&mut txn, id);
    Ok(())
}

// ---------------------------------------------------------------------------
// Attachments — a pointer, never the bytes
// ---------------------------------------------------------------------------

/// Attach a pointer to something that lives elsewhere.
///
/// Never the file itself. Initiative entries are the only place in this store
/// that hold binary blobs, and they carry byte caps because an unbounded upload
/// holds the write mutex every claim waits on. Here the argument is stronger
/// still: bytes in a CRDT log are replayed by every peer that joins, so a map
/// with a PDF in it would get slower to open for everybody, forever.
pub fn add_attachment(
    doc: &Doc,
    node: &str,
    kind: &str,
    name: &str,
    gist: &str,
    reference: &str,
) -> ApiResult<Attachment> {
    if let Some(message) = not_one_of(kind, &ATTACHMENT_KINDS, "attachment kind") {
        return Err(ApiError::validation(
            "validation.mindmap_attachment",
            message,
        ));
    }
    let (name, over) = cap_check(name, MAX_ATTACHMENT_NAME, "attachment name");
    if let Some(message) = over {
        return Err(ApiError::validation(
            "validation.mindmap_attachment",
            message,
        ));
    }
    if name.is_empty() {
        return Err(ApiError::validation(
            "validation.mindmap_attachment",
            "An attachment needs a name — it is what somebody reads before deciding to follow it.",
        ));
    }
    let (gist, over) = cap_check(gist, MAX_ATTACHMENT_GIST, "attachment gist");
    if let Some(message) = over {
        return Err(ApiError::validation(
            "validation.mindmap_attachment",
            message,
        ));
    }
    let (reference, over) = cap_check(reference, MAX_ATTACHMENT_REF, "attachment reference");
    if let Some(message) = over {
        return Err(ApiError::validation(
            "validation.mindmap_attachment",
            message,
        ));
    }

    let (nodes_map, _) = roots(doc);
    let mut txn = doc.transact_mut();
    let entry = node_map(&txn, &nodes_map, node)?;
    let holder = match entry.get(&txn, "attachments") {
        Some(Out::YMap(existing)) => existing,
        _ => entry.insert(&mut txn, "attachments", MapPrelim::default()),
    };
    if holder.len(&txn) as usize >= MAX_ATTACHMENTS {
        return Err(ApiError::conflict(
            "mindmap.attachments_full",
            format!("A node carries at most {MAX_ATTACHMENTS} attachments."),
        ));
    }

    let id = crate::ids::mindmap_attachment_id();
    let record = holder.insert(&mut txn, id.clone(), MapPrelim::default());
    record.insert(&mut txn, "kind", kind.to_string());
    record.insert(&mut txn, "name", name.clone());
    record.insert(&mut txn, "gist", gist.clone());
    record.insert(&mut txn, "ref", reference.clone());
    entry.insert(&mut txn, "updated_at", now_ms());

    Ok(Attachment {
        id,
        kind: kind.to_string(),
        name,
        gist,
        reference,
    })
}

pub fn delete_attachment(doc: &Doc, node: &str, attachment: &str) -> ApiResult<()> {
    let (nodes_map, _) = roots(doc);
    let mut txn = doc.transact_mut();
    let entry = node_map(&txn, &nodes_map, node)?;
    let Some(Out::YMap(holder)) = entry.get(&txn, "attachments") else {
        return Err(ApiError::not_found("mindmap_attachment", attachment));
    };
    if holder.get(&txn, attachment).is_none() {
        return Err(ApiError::not_found("mindmap_attachment", attachment));
    }
    holder.remove(&mut txn, attachment);
    entry.insert(&mut txn, "updated_at", now_ms());
    Ok(())
}

/// Everything a reader of the map needs, in one read.
///
/// The whole map comes back at once because a canvas cannot draw half a tree —
/// affordable precisely because of the node cap, and a better contract than
/// paging a shape.
pub fn snapshot(doc: &Doc, mindmap: &str) -> (Vec<Value>, Vec<Value>, Vec<DocNode>) {
    let (nodes_map, rels_map) = roots(doc);
    let txn = doc.transact();
    let nodes = normalise(read_nodes(&txn, &nodes_map));
    let live: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let relationships = read_relationships(&txn, &rels_map, &live);
    let rank = ranks(&nodes);
    let ordered = tree_order(&nodes);
    let json_nodes: Vec<Value> = ordered
        .iter()
        .map(|n| n.to_json(mindmap, rank.get(&n.id).copied().unwrap_or(0)))
        .collect();
    let json_rels: Vec<Value> = relationships.iter().map(Relationship::to_json).collect();
    (json_nodes, json_rels, nodes)
}

/// Build a map's document from the rows that predate it.
///
/// The row order is the old global `ORDER BY position, created_at, id`, and each
/// ring's order keys are minted in that sequence — so a map that had been
/// arranged by hand comes back looking exactly as it was left.
///
/// Everything the old model did not have takes its default: `notes` empty,
/// `origin` human (nobody can say otherwise about a node that predates the
/// question), `kind` thought. Placement and promotion links carry across
/// unchanged, because both are facts somebody established and neither is
/// recoverable if dropped.
#[allow(clippy::type_complexity)]
pub fn build_from_legacy(
    rows: &[(
        String,
        Option<String>,
        String,
        Option<f64>,
        Option<f64>,
        Option<String>,
        Option<String>,
        String,
        i64,
        i64,
    )],
) -> Vec<u8> {
    let doc = Doc::new();
    {
        let (nodes_map, _) = roots(&doc);
        let mut txn = doc.transact_mut();

        // One ascending run of keys per ring, handed out in row order.
        let mut per_ring: HashMap<Option<String>, usize> = HashMap::new();
        for (_, parent, ..) in rows {
            *per_ring.entry(parent.clone()).or_insert(0) += 1;
        }
        let mut keys: HashMap<Option<String>, std::vec::IntoIter<String>> = per_ring
            .into_iter()
            .map(|(parent, count)| (parent, fracdex::sequence(count).into_iter()))
            .collect();

        for (
            id,
            parent,
            text,
            x,
            y,
            promoted_kind,
            promoted_id,
            created_by,
            created_at,
            updated_at,
        ) in rows
        {
            let order = keys
                .get_mut(parent)
                .and_then(|run| run.next())
                .unwrap_or_else(fracdex::first);

            let entry = nodes_map.insert(&mut txn, id.clone(), MapPrelim::default());
            match parent {
                Some(parent) => entry.insert(&mut txn, "parent", parent.clone()),
                None => entry.insert(&mut txn, "parent", Any::Null),
            };
            entry.insert(&mut txn, "order", order);
            entry.insert(&mut txn, "title", TextPrelim::new(text.clone()));
            entry.insert(&mut txn, "notes", TextPrelim::new(""));
            match x {
                Some(x) => entry.insert(&mut txn, "x", *x),
                None => entry.insert(&mut txn, "x", Any::Null),
            };
            match y {
                Some(y) => entry.insert(&mut txn, "y", *y),
                None => entry.insert(&mut txn, "y", Any::Null),
            };
            entry.insert(&mut txn, "edge_label", String::new());
            entry.insert(&mut txn, "kind", "thought".to_string());
            entry.insert(&mut txn, "origin", "human".to_string());
            entry.insert(&mut txn, "reviewed", false);
            entry.insert(&mut txn, "icons", ArrayPrelim::from(Vec::<String>::new()));
            entry.insert(&mut txn, "color", String::new());
            entry.insert(&mut txn, "shape", String::new());
            entry.insert(&mut txn, "attachments", MapPrelim::default());
            match promoted_kind {
                Some(kind) => entry.insert(&mut txn, "promoted_kind", kind.clone()),
                None => entry.insert(&mut txn, "promoted_kind", Any::Null),
            };
            match promoted_id {
                Some(target) => entry.insert(&mut txn, "promoted_id", target.clone()),
                None => entry.insert(&mut txn, "promoted_id", Any::Null),
            };
            entry.insert(&mut txn, "document", Any::Null);
            entry.insert(&mut txn, "created_by", created_by.clone());
            entry.insert(&mut txn, "created_at", *created_at);
            entry.insert(&mut txn, "updated_at", *updated_at);
        }
    }
    let txn = doc.transact();
    txn.encode_state_as_update_v1(&yrs::StateVector::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, parent: Option<&str>, order: &str, title: &str) -> DocNode {
        DocNode {
            id: id.to_string(),
            parent: parent.map(str::to_string),
            order: order.to_string(),
            title: title.to_string(),
            notes: String::new(),
            x: None,
            y: None,
            edge_label: String::new(),
            kind: "thought".to_string(),
            origin: "human".to_string(),
            reviewed: false,
            icons: Vec::new(),
            color: String::new(),
            shape: String::new(),
            attachments: Vec::new(),
            promoted_kind: None,
            promoted_id: None,
            document: None,
            created_by: "t".to_string(),
            created_by_user: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn an_orphan_comes_back_to_the_root_rather_than_vanishing() {
        // Somebody pruned the parent while somebody else was working under it.
        // Losing the subtree would be the worse of the two answers.
        let nodes = normalise(vec![node("b", Some("gone"), "V", "still here")]);
        assert_eq!(nodes[0].parent, None);
        assert_eq!(tree_order(&nodes).len(), 1);
    }

    #[test]
    fn a_cycle_is_cut_at_the_same_place_no_matter_who_reads_it() {
        // Two peers each made a legal move; together they made a loop. Neither
        // was wrong, so the repair must be a property of the data.
        let forwards = normalise(vec![
            node("a", Some("b"), "V", "a"),
            node("b", Some("a"), "V", "b"),
        ]);
        let backwards = normalise(vec![
            node("b", Some("a"), "V", "b"),
            node("a", Some("b"), "V", "a"),
        ]);

        let cut_forwards: Vec<&str> = forwards
            .iter()
            .filter(|n| n.parent.is_none())
            .map(|n| n.id.as_str())
            .collect();
        let cut_backwards: Vec<&str> = backwards
            .iter()
            .filter(|n| n.parent.is_none())
            .map(|n| n.id.as_str())
            .collect();

        assert_eq!(
            cut_forwards,
            vec!["a"],
            "the lowest id is the one that moves"
        );
        assert_eq!(
            cut_forwards, cut_backwards,
            "and read order must not matter"
        );
        assert_eq!(tree_order(&forwards).len(), 2, "both nodes survive");
    }

    #[test]
    fn a_longer_cycle_still_resolves_and_keeps_everyone() {
        let nodes = normalise(vec![
            node("c", Some("a"), "V", "c"),
            node("a", Some("b"), "V", "a"),
            node("b", Some("c"), "V", "b"),
        ]);
        assert_eq!(tree_order(&nodes).len(), 3);
        assert_eq!(
            nodes.iter().filter(|n| n.parent.is_none()).count(),
            1,
            "exactly one node is cut loose, not the whole ring"
        );
    }

    #[test]
    fn siblings_sort_by_order_key_then_by_id() {
        let nodes = vec![
            node("n2", None, "V", "second"),
            node("n1", None, "F", "first"),
            // Same key as n2: two peers minted it independently. The id decides.
            node("n3", None, "V", "third"),
        ];
        let ordered: Vec<&str> = tree_order(&nodes).iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ordered, vec!["n1", "n2", "n3"]);
    }

    #[test]
    fn an_unusable_order_key_sorts_last_instead_of_anywhere() {
        // A peer is not a trusted writer. A key this server would never mint
        // must not be able to jump the ring.
        let nodes = vec![
            node("n1", None, "not a key!", "junk"),
            node("n2", None, "V", "real"),
        ];
        let ordered: Vec<&str> = tree_order(&nodes).iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ordered, vec!["n2", "n1"]);
    }

    #[test]
    fn the_wire_position_is_a_rank_among_siblings() {
        let nodes = vec![
            node("a", None, "F", "a"),
            node("b", None, "V", "b"),
            node("a1", Some("a"), "F", "a1"),
            node("a2", Some("a"), "V", "a2"),
        ];
        let rank = ranks(&nodes);
        assert_eq!(rank["a"], 0);
        assert_eq!(rank["b"], 1);
        assert_eq!(rank["a1"], 0, "a child's rank is within its own ring");
        assert_eq!(rank["a2"], 1);
    }

    #[test]
    fn the_outline_keeps_the_two_space_shape_agents_read() {
        let nodes = vec![
            node("api", None, "F", "API"),
            node("ver", Some("api"), "F", "versioning?"),
        ];
        assert_eq!(outline(&nodes, "api"), "- API\n  - versioning?\n");
    }

    #[test]
    fn a_branch_carries_its_whole_subtree() {
        let nodes = vec![
            node("a", None, "F", "a"),
            node("b", None, "V", "b"),
            node("a1", Some("a"), "F", "a1"),
            node("a1x", Some("a1"), "F", "deep"),
        ];
        let ids: Vec<&str> = subtree(&nodes, "a").iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "a1", "a1x"]);
    }

    #[test]
    fn depth_counts_the_first_ring_as_one() {
        let nodes = vec![node("a", None, "F", "a"), node("a1", Some("a"), "F", "a1")];
        assert_eq!(depth_of(&nodes, "a"), 1);
        assert_eq!(depth_of(&nodes, "a1"), 2);
    }

    #[test]
    fn a_node_round_trips_through_the_document() {
        let doc = Doc::new();
        let added = add_nodes(
            &doc,
            &[NodeAdd {
                title: "API".to_string(),
                notes: Some("the long form".to_string()),
                ..Default::default()
            }],
            "tester",
        )
        .expect("add");
        let id = added[0].0.clone();

        let (nodes, rels, _) = snapshot(&doc, "mm-test");
        assert_eq!(nodes.len(), 1);
        assert!(rels.is_empty());
        assert_eq!(nodes[0]["title"], "API");
        assert_eq!(nodes[0]["text"], "API", "the old field name still answers");
        assert_eq!(nodes[0]["notes"], "the long form");
        assert_eq!(nodes[0]["position"], 0);
        assert_eq!(nodes[0]["at"], Value::Null);
        assert_eq!(nodes[0]["promoted"], Value::Null);
        assert_eq!(nodes[0]["id"], id);
    }

    #[test]
    fn a_batch_lands_in_the_order_it_was_sent() {
        let doc = Doc::new();
        let adds: Vec<NodeAdd> = ["API", "integrations", "workflows"]
            .iter()
            .map(|t| NodeAdd {
                title: t.to_string(),
                ..Default::default()
            })
            .collect();
        add_nodes(&doc, &adds, "tester").expect("add");
        let (nodes, _, _) = snapshot(&doc, "mm-test");
        let titles: Vec<&str> = nodes.iter().map(|n| n["title"].as_str().unwrap()).collect();
        assert_eq!(titles, vec!["API", "integrations", "workflows"]);
    }

    #[test]
    fn a_refused_batch_writes_nothing() {
        let doc = Doc::new();
        let adds = vec![
            NodeAdd {
                title: "fine".to_string(),
                ..Default::default()
            },
            NodeAdd {
                title: "x".repeat(MAX_TITLE + 1),
                ..Default::default()
            },
        ];
        assert!(add_nodes(&doc, &adds, "tester").is_err());
        let (nodes, _, _) = snapshot(&doc, "mm-test");
        assert!(nodes.is_empty(), "half a branch must never land");
    }

    #[test]
    fn the_title_cap_now_points_at_notes_as_the_way_out() {
        let err = validate_title(&"x".repeat(MAX_TITLE + 1)).unwrap_err();
        let remedy = err.body.remedy.clone().unwrap_or_default();
        assert!(remedy.contains("notes"), "remedy was: {remedy}");
        assert!(remedy.contains("promote"), "the old way out survives too");
    }

    #[test]
    fn deleting_a_node_takes_its_branch_and_its_cross_links() {
        let doc = Doc::new();
        let added = add_nodes(
            &doc,
            &[
                NodeAdd {
                    title: "keep".to_string(),
                    ..Default::default()
                },
                NodeAdd {
                    title: "doomed".to_string(),
                    ..Default::default()
                },
            ],
            "tester",
        )
        .expect("add");
        let keep = added[0].0.clone();
        let doomed = added[1].0.clone();
        let child = add_nodes(
            &doc,
            &[NodeAdd {
                parent: Some(doomed.clone()),
                title: "under it".to_string(),
                ..Default::default()
            }],
            "tester",
        )
        .expect("child")[0]
            .0
            .clone();

        add_relationship(&doc, &keep, &child, "see also").expect("relationship");
        let (_, rels, _) = snapshot(&doc, "mm-test");
        assert_eq!(rels.len(), 1);

        assert_eq!(delete_node(&doc, &doomed).expect("delete"), 2);
        let (nodes, rels, _) = snapshot(&doc, "mm-test");
        assert_eq!(nodes.len(), 1);
        assert!(
            rels.is_empty(),
            "an edge to a node that is gone is not a fact about anything"
        );
    }

    #[test]
    fn a_node_cannot_be_hung_off_its_own_child() {
        let doc = Doc::new();
        let parent = add_nodes(
            &doc,
            &[NodeAdd {
                title: "parent".to_string(),
                ..Default::default()
            }],
            "tester",
        )
        .expect("add")[0]
            .0
            .clone();
        let child = add_nodes(
            &doc,
            &[NodeAdd {
                parent: Some(parent.clone()),
                title: "child".to_string(),
                ..Default::default()
            }],
            "tester",
        )
        .expect("add")[0]
            .0
            .clone();

        let err = patch_node(
            &doc,
            &parent,
            &NodePatch {
                parent: Some(Some(child)),
                ..Default::default()
            },
            "tester",
        )
        .unwrap_err();
        assert_eq!(err.body.code, "mindmap.cycle");
    }

    #[test]
    fn a_relationship_needs_two_real_and_different_nodes() {
        let doc = Doc::new();
        let a = add_nodes(
            &doc,
            &[NodeAdd {
                title: "a".to_string(),
                ..Default::default()
            }],
            "tester",
        )
        .expect("add")[0]
            .0
            .clone();
        assert!(add_relationship(&doc, &a, &a, "self").is_err());
        assert!(add_relationship(&doc, &a, "mn-nothing", "dangling").is_err());
    }

    #[test]
    fn an_attachment_is_a_pointer_and_comes_back_as_one() {
        let doc = Doc::new();
        let a = add_nodes(
            &doc,
            &[NodeAdd {
                title: "a".to_string(),
                ..Default::default()
            }],
            "tester",
        )
        .expect("add")[0]
            .0
            .clone();
        let att = add_attachment(
            &doc,
            &a,
            "pdf",
            "teardown.pdf",
            "density filters as much as it scares",
            "https://example.invalid/teardown.pdf",
        )
        .expect("attach");

        let (nodes, _, _) = snapshot(&doc, "mm-test");
        assert_eq!(nodes[0]["attachments"][0]["name"], "teardown.pdf");
        assert_eq!(
            nodes[0]["attachments"][0]["ref"],
            "https://example.invalid/teardown.pdf"
        );

        delete_attachment(&doc, &a, &att.id).expect("detach");
        let (nodes, _, _) = snapshot(&doc, "mm-test");
        assert_eq!(nodes[0]["attachments"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn the_depth_cap_refuses_at_the_cap_and_counts_a_batch_parent_correctly() {
        // The cap is checked against the depth of the parent. A parent created
        // earlier in the SAME batch is not in the pre-batch map, and `depth_of`
        // answers 1 for an id it has never seen — so the accounting has to carry
        // the batch's own depths rather than ask the old map.
        //
        // Reaching that through the API means naming an id the batch has not
        // minted yet, which a caller cannot do; this is correctness of the
        // accounting, not a hole somebody can walk through. What IS reachable is
        // the ordinary chain below, and it must refuse at exactly the cap.
        let doc = Doc::new();
        let mut parent: Option<String> = None;
        for level in 1..=(MAX_DEPTH + 4) {
            let result = add_nodes(
                &doc,
                &[NodeAdd {
                    parent: parent.clone(),
                    title: format!("level {level}"),
                    ..Default::default()
                }],
                "t",
            );
            match result {
                Ok(created) => {
                    assert!(level <= MAX_DEPTH, "level {level} should have been refused");
                    parent = Some(created[0].0.clone());
                }
                Err(e) => {
                    assert_eq!(e.body.code, "mindmap.too_deep");
                    assert_eq!(level, MAX_DEPTH + 1, "refused at the wrong level");
                    return;
                }
            }
        }
        panic!("the depth cap never refused");
    }

    #[test]
    fn dragging_a_tall_branch_cannot_bury_it_past_the_cap() {
        // The destination's own depth is only half the question: everything under
        // the moved node comes with it. A 3-tall branch dropped at depth 7 lands
        // its leaf at depth 10 against a cap of 8.
        let doc = Doc::new();
        let mut chain = Vec::new();
        let mut parent: Option<String> = None;
        for level in 0..MAX_DEPTH {
            let created = add_nodes(
                &doc,
                &[NodeAdd {
                    parent: parent.clone(),
                    title: format!("deep {level}"),
                    ..Default::default()
                }],
                "t",
            )
            .expect("chain");
            parent = Some(created[0].0.clone());
            chain.push(created[0].0.clone());
        }

        // A separate branch that is itself three levels tall.
        let tall_root = add_nodes(
            &doc,
            &[NodeAdd {
                title: "tall".to_string(),
                ..Default::default()
            }],
            "t",
        )
        .expect("tall")[0]
            .0
            .clone();
        let mid = add_nodes(
            &doc,
            &[NodeAdd {
                parent: Some(tall_root.clone()),
                title: "middle".to_string(),
                ..Default::default()
            }],
            "t",
        )
        .expect("mid")[0]
            .0
            .clone();
        add_nodes(
            &doc,
            &[NodeAdd {
                parent: Some(mid),
                title: "leaf".to_string(),
                ..Default::default()
            }],
            "t",
        )
        .expect("leaf");

        let err = patch_node(
            &doc,
            &tall_root,
            &NodePatch {
                parent: Some(Some(chain[MAX_DEPTH - 2].clone())),
                ..Default::default()
            },
            "t",
        )
        .unwrap_err();
        assert_eq!(err.body.code, "mindmap.too_deep");
        assert!(
            err.body.message.contains("comes along"),
            "the message should say why: {}",
            err.body.message
        );
    }

    #[test]
    fn an_unusable_order_key_never_decides_the_order() {
        // Two peers must draw the same ring. When both keys are junk the id
        // decides on BOTH sides — comparing junk to junk is an ordering the
        // browser does not reproduce.
        let nodes = vec![
            node("mn-aaa", None, "a0", "written second"),
            node("mn-zzz", None, "B0", "written first"),
        ];
        let ordered: Vec<&str> = tree_order(&nodes).iter().map(|n| n.id.as_str()).collect();
        assert_eq!(
            ordered,
            vec!["mn-aaa", "mn-zzz"],
            "with both keys unusable the id decides, as it does in the browser"
        );
    }

    #[test]
    fn a_hostile_order_key_cannot_take_the_map_down() {
        // These arrive over a socket from a peer that is not a trusted writer.
        // Each one used to panic or overflow the stack inside the lock that
        // guards the replica, killing the map for the life of the process.
        let doc = Doc::new();
        let created = add_nodes(
            &doc,
            &[NodeAdd {
                title: "neighbour".to_string(),
                ..Default::default()
            }],
            "t",
        )
        .expect("seed");
        let id = created[0].0.clone();

        let (nodes_map, _) = roots(&doc);
        for hostile in ["", "Aé", &"z".repeat(100_000), "V0", "not a key"] {
            {
                let mut txn = doc.transact_mut();
                let entry = node_map(&txn, &nodes_map, &id).expect("node");
                entry.insert(&mut txn, "order", hostile.to_string());
            }
            // Both an append and an insert at the front hit the neighbour.
            for position in [None, Some(0)] {
                add_nodes(
                    &doc,
                    &[NodeAdd {
                        title: "beside it".to_string(),
                        position,
                        ..Default::default()
                    }],
                    "t",
                )
                .expect("a hostile neighbour must not stop an ordinary insert");
            }
        }
        let (all, _, _) = snapshot(&doc, "mm-test");
        assert!(all.len() > 1, "the inserts landed");
    }

    #[test]
    fn a_bare_leaf_becomes_a_bullet_and_a_thought_with_substance_becomes_a_document() {
        // The rule that decides whether a converted map is readable. Without it
        // every six-word thought is its own page, and forty nodes convert into
        // forty documents nobody opens.
        let mut api = node("api", None, "F", "API");
        api.notes = "The surface everything else hangs off.".into();
        let mut deep = node("deep", Some("api"), "V", "Versioning");
        deep.notes = "v1 forever, or dated?".into();
        let nodes = vec![
            api,
            node("plain", Some("api"), "F", "idempotent retries"),
            deep,
            node("lonely", None, "V", "just a phrase"),
        ];

        let plan = plan_documents(&nodes, "Payments rebuild");
        let made: Vec<&str> = plan.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(
            made,
            vec!["API", "Versioning"],
            "a leaf with nothing on it is not a document, and neither is a lone phrase"
        );

        assert_eq!(plan[0].path, "Payments rebuild");
        assert_eq!(
            plan[1].path, "Payments rebuild/API",
            "ancestry becomes the folder path, so the thinking's shape survives"
        );

        assert_eq!(
            plan[0].blocks,
            vec![
                super::super::prose::Block::Paragraph(
                    "The surface everything else hangs off.".into()
                ),
                super::super::prose::Block::Bullets(vec!["idempotent retries".into()]),
            ],
            "the plain leaf lands as a bullet inside its parent"
        );
    }

    #[test]
    fn a_title_that_is_not_a_folder_name_is_made_into_one() {
        let mut child = node("b", Some("a"), "F", "under it");
        // Notes, so it is a document in its own right and therefore HAS a path.
        child.notes = "so that it becomes a document".into();
        let nodes = vec![node("a", None, "F", "billing / invoicing"), child];
        let plan = plan_documents(&nodes, "");
        assert_eq!(
            plan[0].path, "",
            "the top level is an empty path, not a slash"
        );
        assert!(
            !plan[1].path.contains("billing / invoicing"),
            "a slash in a title would split the path: {}",
            plan[1].path
        );
        assert!(plan[1].path.starts_with("billing - invoicing"));
    }

    #[test]
    fn a_map_converted_twice_remembers_what_it_already_made() {
        let mut a = node("a", None, "F", "API");
        a.notes = "something".into();
        a.document = Some("doc-already01".into());
        let plan = plan_documents(&[a], "root");
        assert_eq!(plan[0].existing.as_deref(), Some("doc-already01"));
    }

    #[test]
    fn a_section_keeps_its_prose_inside_the_node() {
        // The map and the document are two renderings of one thing, so a
        // section's text lives IN its node — not in a document row copied from
        // it, which is what left one paragraph in two places that disagreed.
        let doc = Doc::new();
        let created = add_nodes(
            &doc,
            &[NodeAdd {
                title: "API".to_string(),
                notes: Some("The surface everything hangs off.\nAnd a second line.".to_string()),
                by_user: Some("usr-ada".to_string()),
                ..Default::default()
            }],
            "human:ada",
        )
        .expect("add");
        let id = created[0].0.clone();

        let (nodes, _, raw) = snapshot(&doc, "mm-test");
        assert_eq!(
            nodes[0]["notes"], "The surface everything hangs off.\nAnd a second line.",
            "read back as plain text, one line per block"
        );
        assert_eq!(
            nodes[0]["created_by_user"], "usr-ada",
            "the PERSON, not only the actor string"
        );
        assert_eq!(raw[0].created_by, "human:ada");

        // And it is a fragment, which is what the document editor binds to.
        let (nodes_map, _) = roots(&doc);
        let txn = doc.transact();
        let entry = node_map(&txn, &nodes_map, &id).expect("node");
        assert!(
            matches!(entry.get(&txn, PROSE_KEY), Some(Out::YXmlFragment(_))),
            "a section's prose must be a fragment, or no editor can bind to it"
        );
    }

    #[test]
    fn writing_notes_over_the_api_writes_the_section_prose() {
        let doc = Doc::new();
        let id = add_nodes(
            &doc,
            &[NodeAdd {
                title: "API".to_string(),
                ..Default::default()
            }],
            "t",
        )
        .expect("add")[0]
            .0
            .clone();

        patch_node(
            &doc,
            &id,
            &NodePatch {
                notes: Some("Decided: v1 forever.".to_string()),
                ..Default::default()
            },
            "t",
        )
        .expect("patch");

        let (nodes, _, _) = snapshot(&doc, "mm-test");
        assert_eq!(nodes[0]["notes"], "Decided: v1 forever.");
    }

    #[test]
    fn a_map_written_before_prose_is_moved_into_it_once() {
        // Nodes used to carry `notes` as a plain Y.Text. Leaving both would be
        // exactly the two-places-for-one-paragraph the design removes.
        let doc = Doc::new();
        let id = "mn-legacy01";
        {
            let (nodes_map, _) = roots(&doc);
            let mut txn = doc.transact_mut();
            let entry = nodes_map.insert(&mut txn, id.to_string(), MapPrelim::default());
            entry.insert(&mut txn, "parent", Any::Null);
            entry.insert(&mut txn, "order", "V".to_string());
            entry.insert(&mut txn, "title", TextPrelim::new("API"));
            entry.insert(&mut txn, "notes", TextPrelim::new("written the old way"));
        }

        assert!(ensure_prose(&doc), "the first run moves it");
        let (nodes, _, _) = snapshot(&doc, "mm-test");
        assert_eq!(nodes[0]["notes"], "written the old way", "nothing is lost");

        {
            let (nodes_map, _) = roots(&doc);
            let txn = doc.transact();
            let entry = node_map(&txn, &nodes_map, id).expect("node");
            assert!(matches!(
                entry.get(&txn, PROSE_KEY),
                Some(Out::YXmlFragment(_))
            ));
            assert!(
                entry.get(&txn, "notes").is_none(),
                "the old field goes, so nothing can write where nothing reads"
            );
        }

        assert!(!ensure_prose(&doc), "and running it again changes nothing");
    }

    #[test]
    fn placement_is_kept_and_can_be_handed_back_to_the_layout() {
        let doc = Doc::new();
        let a = add_nodes(
            &doc,
            &[NodeAdd {
                title: "a".to_string(),
                ..Default::default()
            }],
            "tester",
        )
        .expect("add")[0]
            .0
            .clone();

        patch_node(
            &doc,
            &a,
            &NodePatch {
                at: Some(Some((120.5, -40.0))),
                ..Default::default()
            },
            "tester",
        )
        .expect("place");
        let (nodes, _, _) = snapshot(&doc, "mm-test");
        assert_eq!(nodes[0]["at"]["x"], 120.5);

        patch_node(
            &doc,
            &a,
            &NodePatch {
                at: Some(None),
                ..Default::default()
            },
            "tester",
        )
        .expect("unpin");
        let (nodes, _, _) = snapshot(&doc, "mm-test");
        assert_eq!(nodes[0]["at"], Value::Null);
    }
}
