import { act, fireEvent, render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { Awareness } from 'y-protocols/awareness'
import type { WebsocketProvider } from 'y-websocket'
import type { Editor } from '@tiptap/react'
import * as Y from 'yjs'
import { createNode, nodesMap, proseOf, setTitle } from '@/lib/mindmap-crdt'
import { createStructureHistory } from '@/lib/plan-structure'
import SectionEditor from './SectionEditor'

function setup() {
  const doc = new Y.Doc()
  const a = createNode(doc, { title: 'A', parent: null, by: 'test' })!
  const b = createNode(doc, { title: 'B', parent: null, by: 'test' })!
  const history = createStructureHistory(doc)
  const provider = { awareness: new Awareness(doc) } as unknown as WebsocketProvider
  const editors = new Map<string, Editor>()
  const mount = (id: string) => render(<SectionEditor ydoc={doc} sectionId={id} fragment={proseOf(doc,id)!} provider={provider}
    history={history} onSettled={() => {}} display="Ada" color="#123456" canWrite label={id} onEditor={editor => { if (editor) editors.set(id,editor) }} />)
  return { doc, a, b, history, provider, editors, mount }
}

describe('one local document undo history', () => {
  it('undoes edit, move, edit and heading rename in chronological order across editors and remounts', () => {
    const { doc,a,b,history,provider,editors,mount } = setup()
    const first = mount(a), second = mount(b)
    act(() => { editors.get(a)!.commands.insertContent('Alpha') })
    act(() => { history.move(a,b,'child') })
    act(() => { editors.get(b)!.commands.insertContent('Beta') })
    act(() => { history.record(() => setTitle(doc,a,'Renamed')) })
    act(() => { expect(history.undo().ok).toBe(true) })
    expect(nodesMap(doc).get(a)!.get('title')!.toString()).toBe('A')
    act(() => { editors.get(a)!.commands.undo() })
    expect(editors.get(b)!.getText()).toBe('')
    act(() => { history.undo() })
    expect(nodesMap(doc).get(a)!.get('parent')).toBe(null)
    first.unmount()
    const remounted = mount(a)
    act(() => { history.undo() })
    expect(editors.get(a)!.getText()).toBe('')
    act(() => { history.redo(); history.redo(); history.redo(); history.redo() })
    expect(editors.get(a)!.getText()).toBe('Alpha')
    expect(editors.get(b)!.getText()).toBe('Beta')
    expect(nodesMap(doc).get(a)!.get('parent')).toBe(b)
    expect(nodesMap(doc).get(a)!.get('title')!.toString()).toBe('Renamed')
    remounted.unmount(); second.unmount(); history.destroy(); provider.awareness.destroy(); doc.destroy()
  })
  it('preserves concurrent remote prose and rejects conflicting remote structure undo', () => {
    const { doc,a,b,history,provider,editors,mount } = setup()
    const first=mount(a)
    act(() => { editors.get(a)!.commands.insertContent('Local') })
    const peer=new Y.Doc(); Y.applyUpdate(peer,Y.encodeStateAsUpdate(doc))
    const text=(proseOf(peer,a)!.get(0) as Y.XmlElement).get(0) as Y.XmlText
    text.insert(text.length,' remote')
    act(() => { Y.applyUpdate(doc,Y.encodeStateAsUpdate(peer)); history.undo() })
    expect(editors.get(a)!.getText()).toContain('remote')
    expect(editors.get(a)!.getText()).not.toContain('Local')
    act(() => { history.move(a,b,'child') })
    Y.applyUpdate(peer,Y.encodeStateAsUpdate(doc)); nodesMap(peer).get(a)!.set('order','remote-order')
    act(() => { Y.applyUpdate(doc,Y.encodeStateAsUpdate(peer)) })
    expect(history.undo()).toEqual({ok:false,error:'changed'})
    first.unmount(); history.destroy(); provider.awareness.destroy(); doc.destroy(); peer.destroy()
  })
  it('separates rapid edits in different focused sections and preserves formatting and caret across remount', () => {
    const {doc,a,b,history,provider,editors,mount}=setup()
    const first=mount(a), second=mount(b)
    act(() => { fireEvent.focus(editors.get(a)!.view.dom); editors.get(a)!.commands.insertContent('Alpha') })
    act(() => { fireEvent.focus(editors.get(b)!.view.dom); editors.get(b)!.commands.insertContent('Beta') })
    act(() => { history.undo() })
    expect(editors.get(a)!.getText()).toBe('Alpha')
    expect(editors.get(b)!.getText()).toBe('')
    act(() => { history.redo(); history.manager.stopCapturing(); editors.get(a)!.commands.setTextSelection({from:2,to:4}); editors.get(a)!.commands.toggleBold(); history.manager.stopCapturing() })
    first.unmount()
    const remount=mount(a)
    act(() => { history.undo() })
    expect(editors.get(a)!.state.selection.from).toBe(2)
    expect(editors.get(a)!.state.selection.to).toBe(4)
    expect(editors.get(a)!.getHTML()).not.toContain('<strong>')
    act(() => { history.redo() })
    expect(editors.get(a)!.getHTML()).toContain('<strong>')
    remount.unmount(); second.unmount(); history.destroy(); provider.awareness.destroy(); doc.destroy()
  })
  it('includes new sections and refuses to remove a section changed remotely', () => {
    const {doc,a,history,provider,editors,mount}=setup()
    let added:string|null=null
    act(() => { added=history.insert(() => createNode(doc,{title:'New',parent:a,by:'Ada'})) })
    const view=mount(added!)
    act(() => { editors.get(added!)!.commands.insertContent('Text'); history.undo() })
    expect(editors.get(added!)!.getText()).toBe('')
    view.unmount()
    act(() => { expect(history.undo().ok).toBe(true) })
    expect(nodesMap(doc).has(added!)).toBe(false)
    act(() => { history.redo(); history.redo() })
    const peer=new Y.Doc(); Y.applyUpdate(peer,Y.encodeStateAsUpdate(doc)); setTitle(peer,added!,'Peer title')
    act(() => { Y.applyUpdate(doc,Y.encodeStateAsUpdate(peer)); history.undo() })
    expect(history.undo()).toEqual({ok:false,error:'changed'})
    history.destroy(); provider.awareness.destroy(); doc.destroy(); peer.destroy()
  })

  it('guards editor redo after a remote destination deletion and clears redo after a new edit', () => {
    const {doc,a,b,history,provider,editors,mount}=setup()
    const view=mount(a)
    act(() => { history.move(a,b,'child'); history.undo() })
    const peer=new Y.Doc(); Y.applyUpdate(peer,Y.encodeStateAsUpdate(doc)); nodesMap(peer).delete(b)
    act(() => { Y.applyUpdate(doc,Y.encodeStateAsUpdate(peer)) })
    act(() => { expect(editors.get(a)!.commands.redo()).toBe(false) })
    expect(nodesMap(doc).get(a)!.get('parent')).toBe(null)
    expect(nodesMap(doc).has(b)).toBe(false)
    act(() => { editors.get(a)!.commands.insertContent('New branch') })
    expect(history.canRedo).toBe(false)
    view.unmount(); history.destroy(); provider.awareness.destroy(); doc.destroy(); peer.destroy()
  })

  it('groups per-character typing despite sibling decoration-only transactions', () => {
    const {doc,a,b,history,provider,editors,mount}=setup()
    const paragraph=new Y.XmlElement('paragraph'); paragraph.insert(0,[new Y.XmlText('Existing sibling')]); proseOf(doc,b)!.insert(0,[paragraph])
    const first=mount(a), second=mount(b)
    history.manager.clear()
    act(() => {
      fireEvent.focus(editors.get(a)!.view.dom)
      for (const character of 'Typing') {
        editors.get(a)!.commands.insertContent(character)
        const sibling=editors.get(b)!
        sibling.view.dispatch(sibling.state.tr.setMeta('addToHistory',false))
      }
    })
    expect(history.manager.undoStack).toHaveLength(1)
    act(() => { history.undo() })
    expect(editors.get(a)!.getText()).toBe('')
    expect(editors.get(b)!.getText()).toBe('Existing sibling')
    first.unmount(); second.unmount(); history.destroy(); provider.awareness.destroy(); doc.destroy()
  })

})
