import { describe, it, expect } from 'vitest'
import {
  compareVersions,
  mergeHistoryPage,
  type VersionPage,
  type VersionDetail,
  type SavedSection,
} from './spec-history'
const section = (
  id: string,
  extra: Partial<SavedSection> = {},
): SavedSection => ({
  id,
  parent: null,
  position: 0,
  title: id,
  notes: 'Words',
  prose_xml: '<paragraph>Words</paragraph>',
  edge_label: '',
  kind: 'thought',
  color: '',
  shape: '',
  icons: [],
  attachments: [],
  at: null,
  ...extra,
})
const version = (nodes: SavedSection[]): VersionDetail => ({
  version: 1,
  kind: 'save',
  recorded_at: '',
  recorded_by: 'docsync',
  checkpoints: [],
  nodes,
  relationships: [],
})
describe('saved specification comparisons', () => {
  it('tracks additions, removals and moves by stable section identity', () => {
    const changes = compareVersions(
      version([section('a'), section('b')]),
      version([section('a', { parent: 'c', position: 1 }), section('c')]),
    )
    expect(changes.map((c) => [c.id, c.kind])).toEqual([
      ['a', 'changed'],
      ['b', 'removed'],
      ['c', 'added'],
    ])
    expect(changes[0]!.changed).toEqual(['parent', 'position'])
  })
  it('detects rich formatting and attachments even when plain text is unchanged', () => {
    const a = version([section('a')]),
      b = version([
        section('a', {
          prose_xml: '<paragraph><strong>Words</strong></paragraph>',
          attachments: [{ ref: 'x' }],
        }),
      ])
    expect(compareVersions(a, b)[0]!.changed).toEqual([
      'prose_xml',
      'attachments',
    ])
    expect(compareVersions(a, a)).toEqual([])
  })
})

it('keeps older pages across live refreshes without skipping an intervening gap', () => {
  const page = (ids: number[], cursor: number | null): VersionPage => ({
    items: ids.map((id) => ({ ...version([]), version: id })),
    head: ids[0] ?? 0,
    total: 20,
    next_cursor: cursor,
  })
  const current = page([5, 4, 3, 2], 2)
  expect(
    mergeHistoryPage(current, page([7, 6, 5], 5)).items.map((v) => v.version),
  ).toEqual([7, 6, 5, 4, 3, 2])
  expect(mergeHistoryPage(current, page([10, 9, 8], 8)).next_cursor).toBe(8)
  expect(
    mergeHistoryPage(current, page([10, 9, 8], 8)).items.map((v) => v.version),
  ).toEqual([10, 9, 8])
})
