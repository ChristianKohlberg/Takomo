// A reader must not mint block ids.
//
// `BlockId` mints them from an `appendTransaction`, which `editable: false` does
// not stop — so a read-only viewer opening a section with an unnumbered block, or
// two blocks sharing an id (the ordinary result of a concurrent split), changed
// the shared document by looking at it. This shipped without a test; an
// independent reviewer found it by writing one.
import { describe, expect, it } from 'vitest'
import { Editor } from '@tiptap/core'
import StarterKit from '@tiptap/starter-kit'
import Collaboration from '@tiptap/extension-collaboration'
import * as Y from 'yjs'
import { BlockId } from './block-id'

/**
 * Mount, then dispatch a transaction — which is what makes this test test
 * anything.
 *
 * `appendTransaction` only runs when a transaction happens, and in the app one
 * always does: the highlight effect dispatches on mount. A version of this file
 * without the dispatch passed for BOTH the reader and the writer, which is the
 * same "passes by construction" failure that let three broken fixes through.
 */
function mountAndNudge(doc: Y.Doc, fragment: Y.XmlFragment, canWrite: boolean) {
  const editor = mount(doc, fragment, canWrite)
  editor.view.dispatch(editor.state.tr.setMeta('nudge', true))
  return editor
}

function mount(doc: Y.Doc, fragment: Y.XmlFragment, canWrite: boolean) {
  return new Editor({
    extensions: [
      StarterKit.configure({ undoRedo: false }),
      Collaboration.configure({ document: doc, fragment }),
      BlockId.configure({ canWrite }),
    ],
  })
}

/** A paragraph carrying NO id — what an older client or a split leaves behind. */
function unnumbered(doc: Y.Doc): Y.XmlFragment {
  const frag = doc.getXmlFragment('prose')
  const p = new Y.XmlElement('paragraph')
  p.insert(0, [new Y.XmlText('Leaning to v1-forever.')])
  frag.insert(0, [p])
  return frag
}

describe('BlockId', () => {
  it('does not write to the shared document for a reader', () => {
    const doc = new Y.Doc()
    const frag = unnumbered(doc)
    let updates = 0
    doc.on('update', () => { updates += 1 })
    const editor = mountAndNudge(doc, frag, false)
    expect(updates, 'a reader changed the plan by opening it').toBe(0)
    editor.destroy()
  })

  it('still mints them for a writer, which is what keeps blocks addressable', () => {
    const doc = new Y.Doc()
    const frag = unnumbered(doc)
    const editor = mountAndNudge(doc, frag, true)
    expect(frag.toString()).toContain('blk_')
    editor.destroy()
  })

  it('defaults to not minting, so a caller that forgets gets the safe behaviour', () => {
    const doc = new Y.Doc()
    const frag = unnumbered(doc)
    let updates = 0
    doc.on('update', () => { updates += 1 })
    const editor = new Editor({
      extensions: [
        StarterKit.configure({ undoRedo: false }),
        Collaboration.configure({ document: doc, fragment: frag }),
        BlockId,
      ],
    })
    editor.view.dispatch(editor.state.tr.setMeta('nudge', true))
    expect(updates).toBe(0)
    editor.destroy()
  })
})
