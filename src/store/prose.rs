//! Writing a document's FIRST prose, and only its first.
//!
//! The standing rule in this codebase is that Rust reads the CRDT and the
//! browser applies changes to it — `markdown → ProseMirror` needs the editor's
//! exact schema and only the editor has it. That rule is about *editing*: an
//! agent's ops land as a proposal precisely so nobody's live text is rewritten
//! by a process that does not know the schema.
//!
//! This is the one case the rule does not cover. A document made by converting
//! a mindmap has no live text, no reader, and nothing to merge with — it does
//! not exist until this runs. So it is written here, in the small subset of
//! blocks `docprops::read_blocks` already round-trips: a paragraph and a bullet
//! list, nested the way ProseMirror nests them (`bulletList > listItem >
//! paragraph`). Nothing else is generated, and nothing here ever touches a
//! document that already has content.

use yrs::{Doc, ReadTxn, Transact, Xml, XmlElementPrelim, XmlFragment, XmlTextPrelim};

/// The field a Tiptap document keeps its prose in. Must match
/// `docprops::PROSE_FIELD` and the editor's `field: 'prose'`.
const PROSE_FIELD: &str = "prose";

/// One block of a freshly made document.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Paragraph(String),
    Bullets(Vec<String>),
}

/// Build the single update that IS a new document's opening content.
///
/// A Yjs document's whole state serialises as one ordinary update, so this
/// returns exactly what `append_collab_update` wants — the same shape a
/// compaction writes.
pub fn initial_update(blocks: &[Block]) -> Vec<u8> {
    let doc = Doc::new();
    let frag = doc.get_or_insert_xml_fragment(PROSE_FIELD);
    let mut txn = doc.transact_mut();
    for block in blocks {
        match block {
            Block::Paragraph(text) => {
                let el = frag.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
                el.insert_attribute(&mut txn, "id", crate::ids::block_id());
                el.push_back(&mut txn, XmlTextPrelim::new(text.as_str()));
            }
            Block::Bullets(items) => {
                let list = frag.push_back(&mut txn, XmlElementPrelim::empty("bulletList"));
                list.insert_attribute(&mut txn, "id", crate::ids::block_id());
                for item in items {
                    // `bulletList > listItem > paragraph > text` is ProseMirror's
                    // own nesting. Flattening it to `bulletList > text` reads back
                    // correctly through `read_blocks` and renders as nothing in the
                    // editor, which is the failure that looks like success.
                    let li = list.push_back(&mut txn, XmlElementPrelim::empty("listItem"));
                    let para = li.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
                    para.insert_attribute(&mut txn, "id", crate::ids::block_id());
                    para.push_back(&mut txn, XmlTextPrelim::new(item.as_str()));
                }
            }
        }
    }
    drop(txn);
    let txn = doc.transact();
    txn.encode_state_as_update_v1(&yrs::StateVector::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::updates::decoder::Decode;

    /// Read it back the way the rest of the system does, so a shape that only
    /// this file understands cannot pass.
    fn read_back(update: &[u8]) -> Vec<(String, String, Vec<String>)> {
        let doc = Doc::new();
        {
            let mut txn = doc.transact_mut();
            txn.apply_update(yrs::Update::decode_v1(update).expect("decode"))
                .expect("apply");
        }
        let frag = doc.get_or_insert_xml_fragment(PROSE_FIELD);
        let txn = doc.transact();
        crate::api::docprops::read_blocks(&txn, &frag)
            .into_iter()
            .map(|b| (b.kind, b.text, b.items))
            .collect()
    }

    #[test]
    fn a_paragraph_round_trips_through_the_reader_every_agent_uses() {
        let update =
            initial_update(&[Block::Paragraph("Where the billing work came from.".into())]);
        let blocks = read_back(&update);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].0, "paragraph");
        assert_eq!(blocks[0].1, "Where the billing work came from.");
    }

    #[test]
    fn bullets_come_back_as_items_rather_than_as_one_run_of_text() {
        let update = initial_update(&[Block::Bullets(vec![
            "versioning: v1 forever, or dated?".into(),
            "idempotent retries on capture".into(),
        ])]);
        let blocks = read_back(&update);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].0, "bulletList");
        assert_eq!(
            blocks[0].2,
            vec![
                "versioning: v1 forever, or dated?".to_string(),
                "idempotent retries on capture".to_string()
            ],
            "each bullet must be its own item — a flattened list reads as one line"
        );
    }

    #[test]
    fn every_block_carries_an_id_so_an_agent_can_address_it() {
        let update = initial_update(&[
            Block::Paragraph("first".into()),
            Block::Bullets(vec!["a".into()]),
        ]);
        let doc = Doc::new();
        {
            let mut txn = doc.transact_mut();
            txn.apply_update(yrs::Update::decode_v1(&update).expect("decode"))
                .expect("apply");
        }
        let frag = doc.get_or_insert_xml_fragment(PROSE_FIELD);
        let txn = doc.transact();
        for block in crate::api::docprops::read_blocks(&txn, &frag) {
            assert!(
                block.id.starts_with("blk_"),
                "a block with no id cannot be proposed against: {} {}",
                block.kind,
                block.text
            );
        }
    }

    #[test]
    fn an_empty_conversion_produces_an_empty_document_rather_than_junk() {
        let update = initial_update(&[]);
        assert!(read_back(&update).is_empty());
    }
}
