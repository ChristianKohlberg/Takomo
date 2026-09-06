// The per-project conventions a board admin can set.
//
// Five endpoints, each a PUT of one concern. They are separate routes because
// they are separately authorized and separately validated — but the claim-lease
// pair is ONE call even when only one half changed, because the endpoint
// validates them together and sending half of an invalid pair would 422 naming a
// number the admin never touched.
import { api } from './api'
import { charCount } from './format'
import { DEFAULT_DOCUMENT_APPEARANCE, sameDocumentAppearance, validDocumentAppearance, type DocumentAppearance } from './document-appearance'

export interface ProjectSettings {
  /** Human-facing language for ask-a-human questions. */
  language: string
  /** House style for agent-written text. */
  style: string
  /** Answer-link lifetime, seconds. '' = project default. */
  ttl: string
  claimTtl: string
  maxClaimTtl: string
  documentAppearance?: DocumentAppearance
}

export const STYLE_MAX = 2000

const json = { 'Content-Type': 'application/json' }

function put(token: string, path: string, body: unknown) {
  return api(token, path, { method: 'PUT', headers: json, body: JSON.stringify(body) })
}

/** A number field: '' means "unset it", which is a null on the wire, not a 0. */
const num = (v: string) => (v === '' ? null : Number(v))

/**
 * Save only what changed, one call per changed concern.
 *
 * Sequential, not parallel: SQLite is a single writer and two PUTs on the same
 * project row would serialize anyway — this way a failure reports which field it
 * was on instead of an ambiguous rejection.
 */
export async function saveProjectSettings(
  token: string,
  project: string,
  next: ProjectSettings,
  orig: ProjectSettings,
): Promise<number> {
  const base = `/projects/${encodeURIComponent(project)}`
  const calls: [string, unknown][] = []

  if (next.language.trim() !== orig.language.trim()) {
    calls.push([`${base}/language`, { language: next.language.trim() || null }])
  }
  if (next.style.trim() !== orig.style.trim()) {
    calls.push([`${base}/style`, { style_guide: next.style.trim() || null }])
  }
  if (next.ttl !== orig.ttl) {
    calls.push([`${base}/answer-link-ttl`, { ttl_seconds: num(next.ttl) }])
  }
  if (next.claimTtl !== orig.claimTtl || next.maxClaimTtl !== orig.maxClaimTtl) {
    calls.push([
      `${base}/claim-ttl`,
      { ttl_seconds: num(next.claimTtl), max_ttl_seconds: num(next.maxClaimTtl) },
    ])
  }

  if (!sameDocumentAppearance(next.documentAppearance, orig.documentAppearance)) {
    const appearance = next.documentAppearance ?? DEFAULT_DOCUMENT_APPEARANCE
    if (!validDocumentAppearance(appearance)) throw new Error('Invalid document appearance values.')
    calls.push([`${base}/document-appearance`, appearance])
  }

  for (const [path, body] of calls) await put(token, path, body)
  return calls.length
}

export interface ProjectMeta {
  id: string
  name?: string
  question_language?: string | null
  style_guide?: string | null
  answer_link_ttl_seconds?: number | null
  claim_ttl_seconds?: number | null
  max_claim_ttl_seconds?: number | null
  document_appearance?: DocumentAppearance
}

export function settingsFrom(p: ProjectMeta | undefined): ProjectSettings {
  return {
    documentAppearance: { template: p?.document_appearance?.template ?? 'balanced', overrides: { ...p?.document_appearance?.overrides } },
    language: p?.question_language ?? '',
    style: p?.style_guide ?? '',
    ttl: p?.answer_link_ttl_seconds != null ? String(p.answer_link_ttl_seconds) : '',
    claimTtl: p?.claim_ttl_seconds != null ? String(p.claim_ttl_seconds) : '',
    maxClaimTtl: p?.max_claim_ttl_seconds != null ? String(p.max_claim_ttl_seconds) : '',
  }
}

/** '' when it may be saved, otherwise the reason to show. */
export function saveBlockReason(
  s: ProjectSettings,
  readOnly: boolean,
  words: { readOnly: string; over: string; appearanceInvalid?: string },
): string {
  if (readOnly) return words.readOnly
  if (s.documentAppearance && !validDocumentAppearance(s.documentAppearance)) return words.appearanceInvalid ?? 'Invalid document appearance values.'
  if (charCount(s.style.trim()) > STYLE_MAX) return words.over
  return ''
}
