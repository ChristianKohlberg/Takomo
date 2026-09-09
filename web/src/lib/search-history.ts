/** Only explicitly submitted query strings are persisted, never results or credentials. */
export const SEARCH_HISTORY_LIMIT = 8
export function searchHistoryKey(userId?: string, project?: string): string | null {
  return userId && project ? `takomo.search-history.v1:${JSON.stringify([userId, project])}` : null
}
export function rememberSearch(history: readonly string[], query: string): string[] {
  const value = query.trim().slice(0, 500)
  if (!value) return [...history]
  return [value, ...history.filter(item => item.toLowerCase() !== value.toLowerCase())].slice(0, SEARCH_HISTORY_LIMIT)
}
export function readSearchHistory(key: string | null): string[] {
  if (!key) return []
  try {
    const value: unknown = JSON.parse(localStorage.getItem(key) ?? '[]')
    if (!Array.isArray(value)) return []
    return value.filter((item): item is string => typeof item === 'string').slice(0, SEARCH_HISTORY_LIMIT)
      .reverse().reduce<string[]>((history, item) => rememberSearch(history, item), [])
  } catch { return [] }
}
export function writeSearchHistory(key: string | null, history: readonly string[]): void {
  if (!key) return
  try {
    if (history.length) localStorage.setItem(key, JSON.stringify(history))
    else localStorage.removeItem(key)
  } catch { /* Storage may be disabled or full; in-memory history still works. */ }
}
