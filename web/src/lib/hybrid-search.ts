import { api } from './api'
import { literalMatches, type TextMatch } from './document-search'
import type { Node } from '@tiptap/pm/model'

export interface SearchResult {
  node_id: string
  title: string
  heading_path: string[]
  excerpt: string
  passage: string
  location?: { ordinal: number; source_hash: string }
  highlights: string[]
  match_kind: 'keyword' | 'semantic' | 'both'
}
export interface SearchResponse {
  results: SearchResult[]
  /** The most sections one response carries. */
  limit: number
  /** Distinct sections among the bounded candidate set, not a global total. */
  candidates: number
  truncated: boolean
  note?: string
  mode: 'hybrid' | 'keyword'
  semantic_status: 'ready' | 'unconfigured' | 'unavailable' | 'indexing' | 'throttled'
  /** `stale`: the source could not be projected; results are from the last good projection. */
  projection: 'current' | 'stale'
  projection_error: string | null
}
export interface SearchStatus {
  passages_indexed: number
  passages_total: number
  pending: number
  /** Last actual full completion for this provider fingerprint; Unix milliseconds. */
  last_synced_at: number | null
  configured: boolean
  queued: number
  running: number
  failed: number
  indexed: number
  total: number
  last_error: string | null
  projection: 'current' | 'stale'
}
export interface EmbeddingSettings {
  provider: 'voyage' | 'openai'
  endpoint: string
  model: string
  dimensions: number
  quiet_seconds: number
  max_wait_seconds: number
  configured: boolean
}
export const searchDocument = (token: string, map: string, query: string, signal?: AbortSignal) =>
  api<SearchResponse>(token, `/mindmaps/${encodeURIComponent(map)}/search?q=${encodeURIComponent(query)}`, { signal })
export const searchStatus = (token: string, map: string, signal?: AbortSignal) =>
  api<SearchStatus>(token, `/mindmaps/${encodeURIComponent(map)}/search/status`, { signal })
export interface SyncResponse extends SearchStatus {
  /** `deferred`: the document kept changing under the projection; nothing was scheduled. */
  sync: 'scheduled' | 'deferred'
  sync_note?: string
}
export const syncSearch = (token: string, map: string) =>
  api<SyncResponse>(token, `/mindmaps/${encodeURIComponent(map)}/search/sync`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' })

/** Highlight original UTF-16 ranges, including overlapping literal tokens. */
export function excerptRanges(result: SearchResult): TextMatch[] {
  if (result.match_kind === 'semantic') return []
  const ranges = result.highlights.flatMap(token => literalMatches(result.excerpt, token)).sort((a, b) => a.from - b.from)
  return ranges.reduce<TextMatch[]>((merged, range) => {
    const last = merged.at(-1)
    if (last && range.from <= last.to) last.to = Math.max(last.to, range.to)
    else merged.push({ ...range })
    return merged
  }, [])
}

/** Exact passage only: synthetic block separators have no document position. */
function passageText(doc: Node): { text: string; positions: number[] } {
  const lines: { text: string; positions: number[] }[] = []
  let line = { text: '', positions: [] as number[] }
  const cut = () => { lines.push(line); line = { text: '', positions: [] } }
  doc.descendants((node, position) => {
    if (!node.isTextblock) return true
    if (line.text) cut()
    node.descendants((child, offset) => {
      if (child.isText) {
        const value = child.text ?? ''
        line.text += value
        for (let i = 0; i < value.length; i++) line.positions.push(position + 1 + offset + i)
      } else if (child.isLeaf) cut()
    })
    return false
  })
  cut()
  // The index drops blank and whitespace-only lines before storing a passage; derive the same string.
  const kept = lines.filter(entry => entry.text.trim() !== '')
  const text = kept.map(entry => entry.text).join('\n')
  const positions = kept.flatMap((entry, index) => index ? [-1, ...entry.positions] : entry.positions)
  return { text, positions }
}
function rangeAt(positions: number[], index: number, passage: string): TextMatch | null {
  if (index < 0 || !passage) return null
  const from = positions[index], end = positions[index + passage.length - 1]
  return from !== undefined && end !== undefined && from >= 0 && end >= from ? { from, to: end + 1 } : null
}
/** Older servers have no locator. Only a unique exact match is safe. */
export function passageRange(doc: Node, passage: string): TextMatch | null {
  const { text, positions } = passageText(doc)
  const index = text.indexOf(passage)
  if (index < 0 || text.indexOf(passage, index + 1) >= 0) return null
  return rangeAt(positions, index, passage)
}
/** Mirrors the existing server paragraph chunking; Unicode scalars, not UTF-16 units. */
export function passageChunks(text: string): string[] {
  const result: string[] = []
  let current = ''
  for (const paragraph of text.split('\n').filter(value => value.trim())) {
    const chars = Array.from(paragraph)
    if (Array.from(current).length + chars.length + 1 > 2000 && current) { result.push(current); current = '' }
    if (chars.length > 2000) {
      for (let offset = 0; offset < chars.length; offset += 2000) result.push(chars.slice(offset, offset + 2000).join(''))
    } else current += (current ? '\n' : '') + paragraph
  }
  if (current) result.push(current)
  return result.length ? result : ['']
}
export async function locatedPassageRange(doc: Node, result: SearchResult): Promise<TextMatch | null> {
  if (!result.location) return passageRange(doc, result.passage)
  const { text, positions } = passageText(doc)
  const chunks = passageChunks(text)
  const { ordinal, source_hash } = result.location
  if (!Number.isInteger(ordinal) || ordinal < 0 || chunks[ordinal] !== result.passage) return null
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(chunks.join('\0')))
  const hash = Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, '0')).join('')
  if (hash !== source_hash) return null
  let cursor = 0, index = -1
  for (let i = 0; i <= ordinal; i++) {
    const chunk = chunks[i]
    if (chunk === undefined) return null
    index = text.indexOf(chunk, cursor)
    if (index < 0) return null
    cursor = index + chunk.length
  }
  return rangeAt(positions, index, result.passage)
}
