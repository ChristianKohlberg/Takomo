import { describe, expect, it } from 'vitest'
import { canonical, savedText, sectionBlocks, wordChanges } from './saved-prose'
import { compareVersions, groupVersions, type VersionDetail, type SavedVersion } from './spec-history'

describe('saved content review', () => {
  it('keeps literal markup as user text and separates blocks without leaking mark attributes', () => {
    expect(savedText([{ tag: 'paragraph', children: [{ text: [{ insert: 'Use <bold> literally', attributes: { bold: {} } }] }] }, { tag: 'paragraph', children: [{ text: [{ insert: 'Next' }] }] }])).toBe('Use <bold> literally\nNext')
  })
  it('projects legacy XML marks as formatted runs of one line, not as lines of their own', () => {
    const blocks = sectionBlocks({ prose_xml: '<paragraph><bold>Important</bold> rest <link href="https://x.test"><italic>here</italic></link></paragraph><paragraph>Next</paragraph>' })!
    expect(savedText(blocks)).toBe('Important rest here\nNext')
    expect(blocks[0]?.children).toEqual([{ text: [
      { insert: 'Important', attributes: { bold: {} } },
      { insert: ' rest ', attributes: null },
      { insert: 'here', attributes: { link: { href: 'https://x.test' }, italic: {} } },
    ] }])
    const [before, after] = wordChanges(savedText(blocks), savedText(sectionBlocks({ prose_xml: '<paragraph><bold>Important</bold> rest here</paragraph><paragraph>Next</paragraph>' })))
    expect(before.some(x => x.changed)).toBe(false)
    expect(after.some(x => x.changed)).toBe(false)
  })
  it('ignores attribute key order while retaining text, child order and mark changes', () => {
    const a = { tag: 'paragraph', attributes: { id: 'a', level: 1 }, children: [{ text: [{ insert: 'same', attributes: { bold: {}, italic: {} } }] }] }
    const b = { children: [{ text: [{ attributes: { italic: {}, bold: {} }, insert: 'same' }] }], attributes: { level: 1, id: 'a' }, tag: 'paragraph' }
    expect(canonical(a)).toBe(canonical(b))
    const version = (structure: unknown, notes: string) => ({ nodes: [{ id: 'node', title: 'Section', prose_structure: structure, notes }], relationships: [] }) as unknown as VersionDetail
    expect(compareVersions(version([a], '<bold>same</bold>'), version([b], '<italic>same</italic>'))).toEqual([])
    expect(compareVersions(version([a], 'same'), version([{ ...b, children: [{ text: [{ insert: 'changed' }] }] }], 'changed'))).toHaveLength(1)
  })
  it('highlights words added and removed while reconstructing both inputs exactly', () => {
    const [before, after] = wordChanges('The invoice is due today.', 'The invoice is due tomorrow.')
    expect(before.filter(x => x.changed).map(x => x.text).join('')).toBe('today.')
    expect(after.filter(x => x.changed).map(x => x.text).join('')).toBe('tomorrow.')
    expect(before.map(x => x.text).join('')).toBe('The invoice is due today.')
    expect(after.map(x => x.text).join('')).toBe('The invoice is due tomorrow.')
    const huge = 'word '.repeat(1000)
    expect(wordChanges(huge + 'old', huge + 'new')[1].filter(x => x.changed).map(x => x.text).join('')).toBe('new')
  })
  it('groups consecutive save bursts without hiding or merging checkpoints', () => {
    const item = (version: number, minutes: number, named = false): SavedVersion => ({ version, kind: 'save', recorded_at: new Date(minutes * 60_000).toISOString(), recorded_by: 'Ada', checkpoints: named ? [{ name: 'Agreement', actor: 'Ada', user: null, created_at: '' }] : [] })
    expect(groupVersions([item(5, 30), item(4, 29), item(3, 28, true), item(2, 27), item(1, 10)]).map(g => g.map(v => v.version))).toEqual([[5, 4], [3], [2], [1]])
  })
})
