// What "answered" means, per question kind.
//
// This was inline in inbox.html across `answerType`, `curSel`, `curMulti`,
// `recIsAffirmative`, `answerBlockReason` and `displayVal` — the logic that
// decides whether the primary button may fire. It is extracted here because it
// is pure, because it is the single source of truth (the submit path and the
// button's painted state test the SAME function, so they cannot drift), and
// because none of it could be tested where it lived.
import type { Question } from './questions'

export type AnswerType = 'text' | 'single' | 'multi' | 'bool'

/**
 * How this question is answered:
 *
 *   clarify            free text
 *   choose + multi     a set of options
 *   choose             one option, or a written-in value
 *   confirm / approve  yes or no
 */
export function answerType(q: Question): AnswerType {
  if (q.kind === 'clarify') return 'text'
  if (q.kind === 'choose') return q.multi ? 'multi' : 'single'
  return 'bool'
}

/** An agent's recommendation, read as a yes/no for the two boolean kinds. */
export function recIsAffirmative(q: Question): boolean | null {
  const r = q.recommended
  if (r === true || r === 'yes' || r === 'approve' || r === 'true') return true
  if (r === false || r === 'no' || r === 'reject' || r === 'false') return false
  return null
}

export interface Draft {
  /** Single-select value, free text, or the boolean choice. */
  value?: unknown
  /** Ticked options for a multi-select. */
  multi?: string[]
  /** "Write your own" is active on a single-select choose. */
  customOn?: boolean
  custom?: string
}

/**
 * The current selection, defaulted from the agent's recommendation so a
 * recommended answer arms the button straight away — the reader confirms rather
 * than re-derives.
 */
export function currentValue(q: Question, draft: Draft | undefined): unknown {
  if (draft && 'value' in draft && draft.value !== undefined) return draft.value
  if (answerType(q) === 'bool') return recIsAffirmative(q)
  if (answerType(q) === 'text') return q.recommended ?? ''
  return q.recommended ?? null
}

/** The ticked set, defaulted to `recommended_multi`. */
export function currentMulti(q: Question, draft: Draft | undefined): string[] {
  if (draft?.multi) return draft.multi
  return q.recommended_multi ?? []
}

export interface BlockWords {
  typeFirst: string
  sendFirst: string
}

/**
 * "" when the primary may fire, otherwise the reason to show the reader.
 *
 * `submitAnswer` refuses on this same test, which is what keeps the visual state
 * and the actual refusal from drifting apart.
 */
export function answerBlockReason(q: Question, draft: Draft | undefined, w: BlockWords): string {
  const ty = answerType(q)
  if (ty === 'text') return String(currentValue(q, draft) ?? '').trim() ? '' : w.typeFirst
  if (ty === 'multi') return currentMulti(q, draft).length ? '' : w.sendFirst
  if (ty === 'single' && draft?.customOn) return (draft.custom ?? '').trim() ? '' : w.typeFirst
  return currentValue(q, draft) == null ? w.sendFirst : ''
}

/** The payload `POST /answer` takes for the current draft. */
export function answerPayloadFor(q: Question, draft: Draft | undefined): { value: unknown; custom?: boolean } {
  const ty = answerType(q)
  if (ty === 'multi') return { value: [...currentMulti(q, draft)] }
  if (ty === 'single' && draft?.customOn) return { value: (draft.custom ?? '').trim(), custom: true }
  const v = currentValue(q, draft)
  return { value: ty === 'text' ? String(v ?? '').trim() : v }
}

export interface DisplayWords {
  yes: string
  no: string
}

/** The answer as one short phrase, for the undo snackbar. */
export function displayValue(value: unknown, w: DisplayWords): string {
  if (value === true) return w.yes
  if (value === false) return w.no
  if (Array.isArray(value)) return value.join(', ')
  if (value && typeof value === 'object' && 'value' in value) {
    return String((value as { value: unknown }).value ?? '')
  }
  return String(value ?? '')
}
