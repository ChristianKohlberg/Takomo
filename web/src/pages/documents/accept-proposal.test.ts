// Accepting a proposal has to change the SHARED document, not just the editor.
//
// Three pieces were each tested alone and never together: `applyOps` transforms
// a ProseMirror doc, `decideProposal` records a decision, and `SectionEditor`
// binds to one section's fragment. The composition is where the value is — and
// where the failure would be silent, because an editor whose Collaboration
// binding is wrong still applies the ops perfectly to its own state and shows
// the reviewer exactly what they expected, while the Y.Doc everybody else reads
// never moves. Nothing would throw; the change would just not exist for anyone
// but the person who accepted it.
//
// Driven against a real Y.Doc and a real editor for the reason the neighbouring
// SectionEditor test gives: the claim is that Yjs and Tiptap agree, and a mock
// of either would only prove the mock matched the test.
import { describe, expect, it } from 'vitest'
import * as Y from 'yjs'
import { Editor } from '@tiptap/core'
import StarterKit from '@tiptap/starter-kit'
import Collaboration from '@tiptap/extension-collaboration'

import { BlockId } from '@/lib/block-id'
import { applyOps } from '@/lib/doc-ops'
import { proseTextOf } from '@/lib/mindmap-crdt'

/** A node with prose in it, as `createNode` and the server both write one. */
function section(doc: Y.Doc, id: string, text: string): Y.XmlFragment {
  const nodes = doc.getMap<Y.Map<unknown>>('nodes')
  nodes.set(id, new Y.Map())
  const entry = nodes.get(id) as Y.Map<unknown>
  entry.set('prose', new Y.XmlFragment())
  const frag = entry.get('prose') as Y.XmlFragment
  const p = new Y.XmlElement('paragraph')
  p.setAttribute('id', 'blk_one')
  p.insert(0, [new Y.XmlText(text)])
  frag.insert(0, [p])
  return frag
}

function editorOn(doc: Y.Doc, fragment: Y.XmlFragment): Editor {
  return new Editor({
    extensions: [
      StarterKit.configure({ undoRedo: false }),
      Collaboration.configure({ document: doc, fragment }),
      BlockId,
    ],
  })
}

describe('accepting a proposal', () => {
  it('writes the change into the section everybody else is reading', () => {
    const doc = new Y.Doc()
    const frag = section(doc, 'mn-1', 'Leaning to v1-forever.')
    const editor = editorOn(doc, frag)

    const tr = editor.state.tr
    const { applied, skipped } = applyOps(tr, editor.schema, [
      { op: 'insert_after', id: 'blk_one', markdown: 'A mistake is permanent.' },
    ])
    expect(applied, "one op, one block changed").toBe(1)
    expect(skipped).toEqual([])
    editor.view.dispatch(tr)

    // Read from the Y.Doc, NOT from the editor: the editor would agree with
    // itself even if nothing were shared.
    const shared = proseTextOf(doc, 'mn-1')
    expect(shared).toContain('Leaning to v1-forever.')
    expect(shared).toContain('A mistake is permanent.')

    editor.destroy()
  })

  it('reaches a second peer, which is what makes it everybody\'s copy', () => {
    const a = new Y.Doc()
    const frag = section(a, 'mn-1', 'Leaning to v1-forever.')
    const editor = editorOn(a, frag)

    const b = new Y.Doc()
    Y.applyUpdate(b, Y.encodeStateAsUpdate(a))

    const tr = editor.state.tr
    applyOps(tr, editor.schema, [
      { op: 'insert_after', id: 'blk_one', markdown: 'A mistake is permanent.' },
    ])
    editor.view.dispatch(tr)
    Y.applyUpdate(b, Y.encodeStateAsUpdate(a))

    expect(proseTextOf(b, 'mn-1')).toContain('A mistake is permanent.')
    editor.destroy()
  })

  it('leaves a neighbouring section alone', () => {
    const doc = new Y.Doc()
    const one = section(doc, 'mn-1', 'The API surface.')
    section(doc, 'mn-2', 'Integrations, later.')
    const editor = editorOn(doc, one)

    const tr = editor.state.tr
    applyOps(tr, editor.schema, [
      { op: 'replace', id: 'blk_one', markdown: 'The API surface, decided.' },
    ])
    editor.view.dispatch(tr)

    expect(proseTextOf(doc, 'mn-1')).toContain('decided')
    expect(proseTextOf(doc, 'mn-2')).toBe('Integrations, later.')
    editor.destroy()
  })
})
