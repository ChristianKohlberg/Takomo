import { act, fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import * as Y from 'yjs'
import { SectionReferences, referenceHref } from './SectionReferences'
import { addAttachment, createNode, nodesMap, readAttachments, removeAttachment, updateAttachment } from '@/lib/mindmap-crdt'
const source = { kind: 'link' as const, name: 'Specification', ref: 'https://example.com/spec', gist: 'The source' }
function setup() {
  const doc = new Y.Doc()
  const id = createNode(doc, { title: 'Identity', by: 'Ada', parent: null })!
  return { doc, id }
}
describe('section references', () => {
  it('starts collapsed without mutating shared content and only web URLs become links', () => {
    const { doc, id } = setup()
    addAttachment(doc, id, source)
    addAttachment(doc, id, { ...source, name: 'Code', ref: 'src/auth.ts' })
    addAttachment(doc, id, { ...source, name: 'Unsafe', ref: 'javascript:alert(1)' })
    const update = vi.fn(); doc.on('update', update)
    const view = render(<SectionReferences ydoc={doc} sectionId={id} title="Identity" locale="en" canWrite={false} />)
    expect(view.container.querySelector('details')?.open).toBe(false)
    fireEvent.click(screen.getByText('References (3)'))
    expect(view.container.querySelector('details')?.open).toBe(true)
    expect(screen.getByRole('link', { name: 'Specification' }).getAttribute('href')).toBe(source.ref)
    expect(screen.queryByRole('link', { name: 'Code' })).toBeNull()
    expect(screen.queryByRole('link', { name: 'Unsafe' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Manage references' })).toBeNull()
    expect(update).not.toHaveBeenCalled()
    view.unmount(); doc.destroy()
  })
  it('adds, corrects and removes references using existing attachment data', () => {
    const { doc, id } = setup()
    const change = vi.fn((fn: () => void) => fn())
    const view = render(<SectionReferences ydoc={doc} sectionId={id} title="Identity" locale="en" canWrite onChange={change} />)
    fireEvent.click(screen.getByText('References (0)'))
    fireEvent.click(screen.getByRole('button', { name: 'Manage references' }))
    fireEvent.click(screen.getByRole('button', { name: /Add reference/ }))
    fireEvent.change(screen.getByLabelText('Label'), { target: { value: source.name } })
    fireEvent.change(screen.getByLabelText('URL or repository path'), { target: { value: source.ref } })
    fireEvent.click(screen.getByRole('button', { name: 'Add reference' }))
    const attachmentId = readAttachments(nodesMap(doc).get(id)!)[0]!.id
    fireEvent.click(screen.getByRole('button', { name: 'Edit — Specification' }))
    fireEvent.change(screen.getByLabelText('Label'), { target: { value: 'Updated source' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    expect(readAttachments(nodesMap(doc).get(id)!)[0]).toMatchObject({ id: attachmentId, name: 'Updated source' })
    fireEvent.click(screen.getByRole('button', { name: 'Remove — Updated source' }))
    expect(readAttachments(nodesMap(doc).get(id)!)).toEqual([])
    expect(change).toHaveBeenCalledTimes(3)
    view.unmount(); doc.destroy()
  })
  it('reflects remote additions, edits and removals while folding stays local', () => {
    const { doc, id } = setup()
    const peer = new Y.Doc(); Y.applyUpdate(peer, Y.encodeStateAsUpdate(doc))
    const view = render(<SectionReferences ydoc={doc} sectionId={id} title="Identity" locale="de" canWrite={false} />)
    const added = addAttachment(peer, id, source)!
    act(() => Y.applyUpdate(doc, Y.encodeStateAsUpdate(peer)))
    expect(screen.getByText('Referenzen (1)')).toBeTruthy()
    expect(view.container.querySelector('details')?.open).toBe(false)
    act(() => { updateAttachment(peer, id, added, { ...source, name: 'Remote title' }); Y.applyUpdate(doc, Y.encodeStateAsUpdate(peer)) })
    expect(screen.getByText('Remote title')).toBeTruthy()
    const reloaded = new Y.Doc(); Y.applyUpdate(reloaded, Y.encodeStateAsUpdate(doc))
    expect(readAttachments(nodesMap(reloaded).get(id)!)[0]?.name).toBe('Remote title')
    act(() => { removeAttachment(peer, id, added); Y.applyUpdate(doc, Y.encodeStateAsUpdate(peer)) })
    expect(screen.getByText('Referenzen (0)')).toBeTruthy()
    view.unmount(); doc.destroy(); peer.destroy(); reloaded.destroy()
  })
  it('preserves concurrent additions and edits to different references', () => {
    const { doc, id } = setup()
    const original = addAttachment(doc, id, source)!
    const peer = new Y.Doc(); Y.applyUpdate(peer, Y.encodeStateAsUpdate(doc))
    addAttachment(doc, id, { ...source, name: 'Local addition' })
    addAttachment(peer, id, { ...source, name: 'Remote addition' })
    updateAttachment(peer, id, original, { ...source, name: 'Corrected original' })
    const localUpdate = Y.encodeStateAsUpdate(doc)
    const remoteUpdate = Y.encodeStateAsUpdate(peer)
    Y.applyUpdate(doc, remoteUpdate); Y.applyUpdate(peer, localUpdate)
    expect(readAttachments(nodesMap(doc).get(id)!)).toEqual(readAttachments(nodesMap(peer).get(id)!))
    expect(readAttachments(nodesMap(doc).get(id)!).map(item => item.name)).toEqual(['Corrected original', 'Local addition', 'Remote addition'])
    doc.destroy(); peer.destroy()
  })
  it('closes the editing form when write access is revoked', () => {
    const { doc, id } = setup()
    const props = { ydoc: doc, sectionId: id, title: 'Identity', locale: 'en' as const }
    const view = render(<SectionReferences {...props} canWrite />)
    fireEvent.click(screen.getByText('References (0)'))
    fireEvent.click(screen.getByRole('button', { name: 'Manage references' }))
    fireEvent.click(screen.getByRole('button', { name: /Add reference/ }))
    view.rerender(<SectionReferences {...props} canWrite={false} />)
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(readAttachments(nodesMap(doc).get(id)!)).toEqual([])
    view.unmount(); doc.destroy()
  })
  it.each(['javascript:alert(1)', 'data:text/html,test', '//example.com', 'file:///etc/passwd', 'https://user:pass@example.com', 'docs/permissions.md', 'https://'])('keeps %s as plain text', value => {
    expect(referenceHref(value)).toBeNull()
  })
})
