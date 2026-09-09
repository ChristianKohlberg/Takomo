import { afterEach, describe, expect, it, vi } from 'vitest'
import { readSearchHistory, rememberSearch, searchHistoryKey, writeSearchHistory } from './search-history'
afterEach(() => { localStorage.clear(); vi.restoreAllMocks() })
describe('local recent search privacy and bounds', () => {
  it('scopes only by stable user and project and clears only that scope', () => {
    const a = searchHistoryKey('person-a', 'project-a')!
    const b = searchHistoryKey('person-b', 'project-a')!
    const c = searchHistoryKey('person-a', 'project-b')!
    for (const key of [a, b, c]) writeSearchHistory(key, ['Billing'])
    writeSearchHistory(a, [])
    expect(readSearchHistory(a)).toEqual([])
    expect(readSearchHistory(b)).toEqual(['Billing'])
    expect(readSearchHistory(c)).toEqual(['Billing'])
    expect(searchHistoryKey(undefined, 'project-a')).toBeNull()
    writeSearchHistory(null, ['Machine query'])
    expect(localStorage.length).toBe(2)
  })
  it('keeps eight most recent trimmed queries, deduplicating case without losing spelling', () => {
    let history: string[] = []
    for (let n = 0; n < 10; n++) history = rememberSearch(history, `Query ${n}`)
    history = rememberSearch(history, '  QUERY 4  ')
    expect(history).toEqual(['QUERY 4', 'Query 9', 'Query 8', 'Query 7', 'Query 6', 'Query 5', 'Query 3', 'Query 2'])
    expect(rememberSearch(history, '  ')).toEqual(history)
  })
  it('tolerates unavailable or malformed storage without storing extra fields', () => {
    const key = searchHistoryKey('person', 'project')!
    localStorage.setItem(key, JSON.stringify([{ results: 'private' }, ' valid ', null]))
    expect(readSearchHistory(key)).toEqual(['valid'])
    localStorage.setItem(key, '{broken')
    expect(readSearchHistory(key)).toEqual([])
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('disabled') })
    expect(() => writeSearchHistory(key, ['safe'])).not.toThrow()
  })
})
