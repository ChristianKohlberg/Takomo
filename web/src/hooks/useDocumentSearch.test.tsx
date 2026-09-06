import { act, renderHook } from '@testing-library/react'
import { expect, it } from 'vitest'
import * as Y from 'yjs'
import { useDocumentSearch } from './useDocumentSearch'

it('updates unmounted prose on peer edits without selecting a neighboring section after deletion', () => {
  const doc = new Y.Doc()
  const section = new Y.Map()
  doc.getMap('nodes').set('first', section)
  doc.getMap('nodes').set('second', new Y.Map())
  const fragment = new Y.XmlFragment()
  section.set('prose', fragment)
  const paragraph = new Y.XmlElement('paragraph')
  fragment.insert(0, [paragraph])
  const text = new Y.XmlText('match')
  paragraph.insert(0, [text])
  const nodes = [{ id: 'first', title: 'Title' }, { id: 'second', title: 'match' }]
  const { result } = renderHook(() => useDocumentSearch(doc, nodes))
  act(() => result.current.setQuery('match'))
  expect(result.current.matches).toHaveLength(2)
  expect(result.current.activeMatch?.sectionId).toBe('first')
  const peer = new Y.Doc()
  Y.applyUpdate(peer, Y.encodeStateAsUpdate(doc))
  peer.getMap('nodes').delete('first')
  act(() => Y.applyUpdate(doc, Y.encodeStateAsUpdate(peer)))
  expect(result.current.matches).toHaveLength(1)
  expect(result.current.activeMatch).toBeNull()
  act(() => result.current.next())
  expect(result.current.activeMatch?.sectionId).toBe('second')
  act(() => result.current.clear())
  expect(result.current.matches).toEqual([])
})

it('keeps the same CRDT occurrence active when a collaborator inserts earlier text', () => {
  const doc = new Y.Doc()
  const section = new Y.Map()
  doc.getMap('nodes').set('one', section)
  const fragment = new Y.XmlFragment()
  section.set('prose', fragment)
  const paragraph = new Y.XmlElement('paragraph')
  fragment.insert(0, [paragraph])
  const text = new Y.XmlText('match match')
  paragraph.insert(0, [text])
  const nodes = [{ id: 'one', title: '' }]
  const { result } = renderHook(() => useDocumentSearch(doc, nodes))
  act(() => result.current.setQuery('match'))
  const identity = result.current.activeMatch?.key
  act(() => text.insert(0, 'prefix '))
  expect(result.current.activeMatch?.key).toBe(identity)
  expect(result.current.activeMatch?.from).toBe(8)
  act(() => text.delete(7, 6))
  expect(result.current.matches).toHaveLength(1)
  expect(result.current.activeMatch).toBeNull()
})

it('does not subscribe to document writes while search is empty', () => {
  const doc = new Y.Doc()
  const section = new Y.Map()
  doc.getMap('nodes').set('one', section)
  const nodes = [{ id: 'one', title: 'Title' }]
  let renders = 0
  const { result } = renderHook(() => { renders += 1; return useDocumentSearch(doc, nodes) })
  const before = renders
  act(() => section.set('notes', 'another edit'))
  expect(renders).toBe(before)
  act(() => result.current.setQuery('Title'))
  const searching = renders
  act(() => section.set('notes', 'changed while searching'))
  expect(renders).toBeGreaterThan(searching)
  act(() => result.current.clear())
  const closed = renders
  act(() => section.set('notes', 'changed after closing'))
  expect(renders).toBe(closed)
})
