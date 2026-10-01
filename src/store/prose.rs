//! A section's prose, as plain text.
//!
//! The standing rule in this codebase is that Rust reads the CRDT and changes
//! prose only through a converter held to the editor's schema — the browser's
//! `markdownToNodes`, or its fixture-checked twin `api::proposal_apply` when an
//! agent accepts a proposal. That rule is about *editing*: an agent's ops land
//! as a proposal precisely so nobody's live text is rewritten by a process that
//! does not know the schema.
//!
//! This is the one case the rule does not cover, and the reason has CHANGED —
//! the paragraph here used to justify it by the mindmap-to-document conversion,
//! which was deleted when the plan became the document rather than a copy of it.
//!
//! What keeps it is the notes box: the map offers one plain-text field per node,
//! and `patch_node` writes it through `set_plain_text`. So this DOES touch prose
//! that already has content — it replaces the section wholesale and re-mints
//! every block id, which invalidates a pending proposal addressing those blocks.
//! It is only allowed where nothing else is lost: [`is_plain`] gates it, and a
//! section with headings, lists, tables, marks or references is refused
//! (`conflict.notes_would_flatten`) instead of flattened. Rich prose changes
//! through the document view or a proposal.
//!
//! What it generates is unchanged: the small subset of blocks
//! `docprops::read_blocks` already round-trips — a paragraph and a bullet list,
//! nested the way ProseMirror nests them (`bulletList > listItem > paragraph`).

use yrs::{
    Any, GetString, Map, Out, ReadTxn, TransactionMut, Xml, XmlElementPrelim, XmlFragment,
    XmlFragmentRef, XmlOut, XmlTextPrelim,
};

/// The text of one block, with any nesting flattened.
///
/// Not `get_string`, which serialises the element back to XML and would hand a
/// reader `<paragraph id="blk_x">…</paragraph>` as if it were prose. The same
/// trap `docprops::element_text` documents, and the same answer. References read
/// their target title from this transaction, leaving the stored fallback and
/// all shared prose unchanged.
pub(crate) fn element_text<T: ReadTxn>(txn: &T, el: &yrs::XmlElementRef) -> String {
    if el.tag().as_ref() == "sectionReference" {
        let title = match el.get_attribute(txn, "sectionId") {
            Some(Out::Any(Any::String(id))) => txn
                .get_map(super::mindmapdoc::NODES_FIELD)
                .and_then(|nodes| nodes.get(txn, id.as_ref()))
                .and_then(|entry| match entry {
                    Out::YMap(entry) => Some(match entry.get(txn, "title") {
                        Some(Out::YText(text)) => text.get_string(txn),
                        Some(Out::Any(Any::String(text))) => text.to_string(),
                        _ => String::new(),
                    }),
                    _ => None,
                }),
            _ => None,
        };
        return match title {
            Some(title) if !title.is_empty() => title,
            Some(_) => "Untitled section".to_string(),
            None => {
                let fallback = element_children_text(txn, el);
                format!(
                    "{} (Missing section)",
                    if fallback.is_empty() {
                        "Untitled section"
                    } else {
                        &fallback
                    }
                )
            }
        };
    }
    element_children_text(txn, el)
}

fn element_children_text<T: ReadTxn>(txn: &T, el: &yrs::XmlElementRef) -> String {
    let mut out = String::new();
    for child in el.children(txn) {
        match child {
            XmlOut::Text(text) => out.push_str(&text.get_string(txn)),
            XmlOut::Element(inner) => out.push_str(&element_text(txn, &inner)),
            XmlOut::Fragment(_) => {}
        }
    }
    out
}

/// A fragment's prose as plain text, one line per block.
///
/// This is what a canvas card, an outline and a search read. The map's cards
/// cannot render ProseMirror and should not try: one line of what a section says
/// is the entire job.
pub fn plain_text<T: ReadTxn>(txn: &T, frag: &XmlFragmentRef) -> String {
    let mut lines = Vec::new();
    for node in frag.children(txn) {
        match node {
            XmlOut::Element(el) => {
                let text = element_text(txn, &el);
                if !text.trim().is_empty() {
                    lines.push(text);
                }
            }
            XmlOut::Text(text) => {
                let text = text.get_string(txn);
                if !text.trim().is_empty() {
                    lines.push(text);
                }
            }
            XmlOut::Fragment(_) => {}
        }
    }
    lines.join("\n")
}

/// Whether a fragment holds nothing but plain paragraphs.
///
/// "Plain" is exactly what [`set_plain_text`] writes: top-level `paragraph`
/// elements carrying no attribute but their block `id`, whose only children are
/// text runs without marks. Anything else — a heading, a list, a table, a code
/// block, a collapsible block, a section reference, a hard break, a bold word,
/// a link — is structure the plain-text path cannot carry, so replacing it with
/// [`set_plain_text`] would flatten it. Callers ask this first and refuse
/// rather than flatten; `patch_node` is the one that matters.
///
/// An empty fragment is plain: there is nothing in it to lose.
pub fn is_plain<T: ReadTxn>(txn: &T, frag: &XmlFragmentRef) -> bool {
    use yrs::types::text::YChange;
    use yrs::Text;
    frag.children(txn).all(|child| match child {
        XmlOut::Element(el) => {
            el.tag().as_ref() == "paragraph"
                && el.attributes(txn).all(|(key, _)| key == "id")
                && el.children(txn).all(|inner| match inner {
                    XmlOut::Text(text) => {
                        text.diff(txn, YChange::identity).into_iter().all(|part| {
                            part.attributes
                                .as_ref()
                                .is_none_or(|attrs| attrs.is_empty())
                                && matches!(part.insert, Out::Any(Any::String(_)))
                        })
                    }
                    _ => false,
                })
        }
        XmlOut::Text(_) | XmlOut::Fragment(_) => false,
    })
}

/// Replace a fragment's whole content with these paragraphs.
///
/// A wholesale replace, because this is the path a caller takes when it sends a
/// finished string over the API. Only ever call it on a fragment that
/// [`is_plain`] accepts: on anything richer it would flatten the structure. Somebody typing in the browser edits the same
/// fragment character by character through the editor, which is where the merge
/// actually matters.
pub fn set_plain_text(txn: &mut TransactionMut, frag: &XmlFragmentRef, text: &str) {
    let len = frag.len(txn);
    if len > 0 {
        frag.remove_range(txn, 0, len);
    }
    for line in text.split('\n') {
        if line.trim().is_empty() {
            continue;
        }
        let el = frag.push_back(txn, XmlElementPrelim::empty("paragraph"));
        el.insert_attribute(txn, "id", crate::ids::block_id());
        el.push_back(txn, XmlTextPrelim::new(line));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::{Doc, Out, Transact, XmlFragmentPrelim};

    fn round_trip(text: &str) -> String {
        let doc = Doc::new();
        let frag = doc.get_or_insert_xml_fragment("prose");
        let mut txn = doc.transact_mut();
        set_plain_text(&mut txn, &frag, text);
        drop(txn);
        let txn = doc.transact();
        plain_text(&txn, &frag)
    }

    #[test]
    fn a_section_reads_back_the_way_it_was_written() {
        assert_eq!(
            round_trip("The surface everything hangs off."),
            "The surface everything hangs off."
        );
    }

    #[test]
    fn each_line_is_its_own_block_and_comes_back_as_its_own_line() {
        // A card shows one line of this and the document view renders the
        // blocks. Both need the paragraphs to be paragraphs, not one run.
        assert_eq!(round_trip("first\nsecond\nthird"), "first\nsecond\nthird");
    }

    #[test]
    fn blank_lines_do_not_become_empty_paragraphs() {
        assert_eq!(round_trip("first\n\n\nsecond"), "first\nsecond");
    }

    #[test]
    fn every_block_carries_an_id_so_an_agent_can_address_it() {
        let doc = Doc::new();
        let frag = doc.get_or_insert_xml_fragment("prose");
        let mut txn = doc.transact_mut();
        set_plain_text(&mut txn, &frag, "one\ntwo");
        drop(txn);

        let txn = doc.transact();
        for node in frag.children(&txn) {
            let XmlOut::Element(el) = node else { continue };
            let id = match el.get_attribute(&txn, "id") {
                Some(Out::Any(yrs::Any::String(id))) => id.to_string(),
                other => panic!("a block with no readable id: {other:?}"),
            };
            assert!(
                id.starts_with("blk_"),
                "a block with no id cannot be proposed against: {id:?}"
            );
        }
    }

    #[test]
    fn rewriting_replaces_rather_than_appends() {
        let doc = Doc::new();
        let frag: yrs::XmlFragmentRef = doc.get_or_insert_xml_fragment("prose");
        {
            let mut txn = doc.transact_mut();
            set_plain_text(&mut txn, &frag, "the first thing");
            set_plain_text(&mut txn, &frag, "the second thing");
        }
        let txn = doc.transact();
        assert_eq!(plain_text(&txn, &frag), "the second thing");
        let _ = XmlFragmentPrelim::default();
    }

    #[test]
    fn plain_paragraphs_and_an_empty_section_are_plain() {
        let doc = Doc::new();
        let frag: yrs::XmlFragmentRef = doc.get_or_insert_xml_fragment("prose");
        assert!(is_plain(&doc.transact(), &frag), "empty");
        set_plain_text(&mut doc.transact_mut(), &frag, "one\ntwo");
        assert!(is_plain(&doc.transact(), &frag));
    }

    #[test]
    fn structure_marks_and_references_are_not_plain() {
        use yrs::types::Attrs;
        use yrs::{Text, XmlElementPrelim as El, XmlTextPrelim as T};
        // One case per shape the notes box would lose.
        type Build = fn(&mut yrs::TransactionMut, &yrs::XmlFragmentRef);
        let cases: [(&str, Build); 5] = [
            ("heading", |txn, frag| {
                let h = frag.push_back(txn, El::empty("heading"));
                h.push_back(txn, T::new("Title"));
            }),
            ("table", |txn, frag| {
                frag.push_back(txn, El::empty("table"));
            }),
            ("section reference", |txn, frag| {
                let p = frag.push_back(txn, El::empty("paragraph"));
                p.push_back(txn, El::empty("sectionReference"));
            }),
            ("paragraph attribute", |txn, frag| {
                let p = frag.push_back(txn, El::empty("paragraph"));
                p.insert_attribute(txn, "textAlign", "center");
                p.push_back(txn, T::new("centred"));
            }),
            ("bold mark", |txn, frag| {
                let p = frag.push_back(txn, El::empty("paragraph"));
                let text = p.push_back(txn, T::new(""));
                let mut bold = Attrs::new();
                bold.insert("bold".into(), Any::Map(Default::default()));
                text.insert_with_attributes(txn, 0, "loud", bold);
            }),
        ];
        for (name, build) in cases {
            let doc = Doc::new();
            let frag: yrs::XmlFragmentRef = doc.get_or_insert_xml_fragment("prose");
            {
                let mut txn = doc.transact_mut();
                let p = frag.push_back(&mut txn, El::empty("paragraph"));
                p.push_back(&mut txn, T::new("plain lead"));
                build(&mut txn, &frag);
            }
            assert!(!is_plain(&doc.transact(), &frag), "{name} read as plain");
        }
    }
}
