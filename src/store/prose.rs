//! A section's prose, as plain text.
//!
//! The standing rule in this codebase is that Rust reads the CRDT and the
//! browser applies changes to it — `markdown → ProseMirror` needs the editor's
//! exact schema and only the editor has it. That rule is about *editing*: an
//! agent's ops land as a proposal precisely so nobody's live text is rewritten
//! by a process that does not know the schema.
//!
//! This is the one case the rule does not cover, and the reason has CHANGED —
//! the paragraph here used to justify it by the mindmap-to-document conversion,
//! which was deleted when the plan became the document rather than a copy of it.
//!
//! What keeps it is the notes box: the map offers one plain-text field per node,
//! and `patch_node` writes it through `set_plain_text`. So this DOES touch prose
//! that already has content, which the sentence here used to deny — it replaces
//! the section wholesale and re-mints every block id. That is a real trade and
//! the browser side documents it deliberately: a section's headings and lists,
//! written in `/documents`, do not survive somebody typing in the map's notes
//! box, and a pending proposal addressing those blocks is invalidated. The plain
//! field is the map's whole idea of prose; the structure lives on the other
//! surface.
//!
//! What it generates is unchanged: the small subset of blocks
//! `docprops::read_blocks` already round-trips — a paragraph and a bullet list,
//! nested the way ProseMirror nests them (`bulletList > listItem > paragraph`).

use yrs::{
    GetString, ReadTxn, TransactionMut, Xml, XmlElementPrelim, XmlFragment, XmlFragmentRef, XmlOut,
    XmlTextPrelim,
};

/// The text of one block, with any nesting flattened.
///
/// Not `get_string`, which serialises the element back to XML and would hand a
/// reader `<paragraph id="blk_x">…</paragraph>` as if it were prose. The same
/// trap `docprops::element_text` documents, and the same answer.
fn element_text<T: ReadTxn>(txn: &T, el: &yrs::XmlElementRef) -> String {
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

/// Replace a fragment's whole content with these paragraphs.
///
/// A wholesale replace, because this is the path a caller takes when it sends a
/// finished string over the API. Somebody typing in the browser edits the same
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
}
