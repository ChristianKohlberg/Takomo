import { afterEach, expect, it } from 'vitest'
import { Editor } from '@tiptap/react'
import { TableKit } from '@tiptap/extension-table'
import StarterKit from '@tiptap/starter-kit'
import * as Y from 'yjs'
import { createNode } from './mindmap-crdt'
import { DocumentSectionReference } from './document-section-reference'
import { SectionReferenceTrigger, referenceMatch, insertReferenceMatch, closeReferenceMenu } from './section-reference-trigger'
const editors: Editor[] = []
const docs: Y.Doc[] = []
afterEach(() => { editors.splice(0).forEach(e => e.destroy()); docs.splice(0).forEach(d => d.destroy()) })
function setup(content = '<p>See </p>') {
  const doc = new Y.Doc(); docs.push(doc)
  const id = createNode(doc, { title: 'Billing', parent: null, by: 'test' })!
  const editor = new Editor({ extensions: [StarterKit, TableKit, DocumentSectionReference.configure({ ydoc: doc }), SectionReferenceTrigger], content, parseOptions: { preserveWhitespace: 'full' } })
  editors.push(editor); editor.state.doc.descendants((node, pos) => { if (node.isTextblock) editor.commands.setTextSelection(pos + 1 + node.content.size) })
  return { editor, doc, id }
}
function type(editor: Editor, text: string) {
  const { from, to } = editor.state.selection
  let handled = false
  editor.view.someProp('handleTextInput', fn => { handled = fn(editor.view, from, to, text, () => editor.state.tr.insertText(text)) || false; return handled })
  if (!handled) editor.view.dispatch(editor.state.tr.insertText(text))
}
it('replaces a mid-paragraph query atomically and restores query/caret on undo', () => {
  const { editor, doc, id } = setup()
  type(editor, '@'); type(editor, 'Bill')
  expect(referenceMatch(editor.state)?.query).toBe('Bill')
  expect(insertReferenceMatch(editor, doc, referenceMatch(editor.state)!, id)).toBe(true)
  expect(editor.getText()).toBe('See 1 Billing')
  expect(referenceMatch(editor.state)).toBeNull()
  editor.commands.undo()
  expect(editor.getText()).toBe('See @Bill')
  expect(editor.state.selection.from).toBe(10)
  expect(editor.state.doc.toJSON().content[0].content?.some((n: { type: string }) => n.type === 'sectionReference') ?? false).toBe(false)
})
it.each(['<p>email</p>', '<pre><code>code </code></pre>', '<p><code>code </code></p>'])('keeps email/code @ literal: %s', content => {
  const { editor } = setup(content); type(editor, '@'); expect(referenceMatch(editor.state)).toBeNull()
})
it('works inside normal list prose and Escape leaves literal text without reopening', () => {
  const { editor } = setup('<ul><li><p>See </p></li></ul>')
  editor.commands.setTextSelection(7); type(editor, '@'); expect(referenceMatch(editor.state)).not.toBeNull()
  closeReferenceMenu(editor); type(editor, 'Billing'); expect(referenceMatch(editor.state)).toBeNull()
  expect(editor.getText()).toContain('@Billing')
})
it('rejects stale ranges after remote deletion and does not trigger on pasted/programmatic @', () => {
  const { editor, doc, id } = setup()
  editor.commands.insertContent('@'); expect(referenceMatch(editor.state)).toBeNull()
  type(editor, ' '); type(editor, '@'); type(editor, 'Bill')
  const offered = referenceMatch(editor.state)!
  editor.view.dispatch(editor.state.tr.delete(offered.from, offered.to))
  expect(insertReferenceMatch(editor, doc, offered, id)).toBe(false)
  expect(editor.getText()).toBe('See @ ')
})

it('respects pending inline code marks before the first character', () => {
  const { editor } = setup('<p></p>'); editor.commands.toggleCode(); type(editor, '@'); expect(referenceMatch(editor.state)).toBeNull()
})

it('opens in table-cell prose and searches the same stable section identity', () => {
  const { editor, doc, id } = setup('<table><tbody><tr><td><p>See </p></td></tr></tbody></table>')
  type(editor, '@'); type(editor, '1')
  expect(insertReferenceMatch(editor, doc, referenceMatch(editor.state)!, id)).toBe(true)
  expect(editor.state.doc.firstChild!.type.name).toBe('table')
  expect(editor.getText()).toContain('1 Billing')
})
