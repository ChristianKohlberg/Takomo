import { describe, it, expect } from 'vitest'
import {
  answerBlockReason,
  answerPayloadFor,
  answerType,
  currentMulti,
  currentValue,
  displayValue,
  recIsAffirmative,
} from './answers'
import type { Question, QuestionKind } from './questions'

const W = { typeFirst: 'Type an answer first', sendFirst: 'Pick one first' }

function q(over: Partial<Question> = {}): Question {
  return {
    id: 'q-1',
    project: 'demo',
    ticket: 'demo-1',
    kind: 'clarify',
    mode: 'blocking',
    status: 'open',
    title: 'A question',
    options: [],
    option_notes: [],
    multi: false,
    recommended_multi: [],
    expertise: [],
    created_at: '2026-08-08T00:00:00Z',
    ...over,
  }
}

describe('answerType', () => {
  it.each<[QuestionKind, Partial<Question>, string]>([
    ['clarify', {}, 'text'],
    ['choose', { multi: false }, 'single'],
    ['choose', { multi: true }, 'multi'],
    ['confirm', {}, 'bool'],
    ['approve', {}, 'bool'],
  ])('%s → %s', (kind, over, want) => {
    expect(answerType(q({ kind, ...over }))).toBe(want)
  })
})

describe('recIsAffirmative', () => {
  it('reads the several spellings an agent may use', () => {
    expect(recIsAffirmative(q({ recommended: true }))).toBe(true)
    expect(recIsAffirmative(q({ recommended: 'yes' }))).toBe(true)
    expect(recIsAffirmative(q({ recommended: 'approve' }))).toBe(true)
    expect(recIsAffirmative(q({ recommended: false }))).toBe(false)
    expect(recIsAffirmative(q({ recommended: 'reject' }))).toBe(false)
  })

  it('returns null when there is no recommendation to read', () => {
    expect(recIsAffirmative(q())).toBeNull()
    expect(recIsAffirmative(q({ recommended: 'maybe' }))).toBeNull()
  })
})

describe('defaults come from the recommendation', () => {
  it('preselects a recommended yes/no so the reader confirms rather than re-derives', () => {
    expect(currentValue(q({ kind: 'confirm', recommended: true }), undefined)).toBe(true)
    expect(currentValue(q({ kind: 'approve', recommended: 'reject' }), undefined)).toBe(false)
  })

  it('pre-fills recommended text', () => {
    expect(currentValue(q({ kind: 'clarify', recommended: 'reuse the key' }), undefined)).toBe(
      'reuse the key',
    )
  })

  it('ticks a recommended multi set', () => {
    const question = q({ kind: 'choose', multi: true, recommended_multi: ['a', 'b'] })
    expect(currentMulti(question, undefined)).toEqual(['a', 'b'])
  })

  it('lets the draft override the recommendation', () => {
    const question = q({ kind: 'confirm', recommended: true })
    expect(currentValue(question, { value: false })).toBe(false)
  })
})

describe('answerBlockReason — the single source of truth for the primary button', () => {
  it('requires non-empty text for clarify', () => {
    expect(answerBlockReason(q({ kind: 'clarify' }), undefined, W)).toBe(W.typeFirst)
    expect(answerBlockReason(q({ kind: 'clarify' }), { value: '  ' }, W)).toBe(W.typeFirst)
    expect(answerBlockReason(q({ kind: 'clarify' }), { value: 'because' }, W)).toBe('')
    // A recommendation arms it immediately.
    expect(answerBlockReason(q({ kind: 'clarify', recommended: 'reuse' }), undefined, W)).toBe('')
  })

  it('requires at least one ticked option for a multi choose', () => {
    const m = q({ kind: 'choose', multi: true, options: ['a', 'b'] })
    expect(answerBlockReason(m, undefined, W)).toBe(W.sendFirst)
    expect(answerBlockReason(m, { multi: ['a'] }, W)).toBe('')
    // A recommended set arms it straight away.
    expect(answerBlockReason(q({ ...m, recommended_multi: ['b'] }), undefined, W)).toBe('')
  })

  it('requires text when "write your own" is active on a single choose', () => {
    const s = q({ kind: 'choose', options: ['a', 'b'], recommended: 'a' })
    expect(answerBlockReason(s, undefined, W)).toBe('')
    expect(answerBlockReason(s, { customOn: true, custom: '' }, W)).toBe(W.typeFirst)
    expect(answerBlockReason(s, { customOn: true, custom: 'other' }, W)).toBe('')
  })

  it('requires a pick for confirm/approve with no recommendation', () => {
    expect(answerBlockReason(q({ kind: 'confirm' }), undefined, W)).toBe(W.sendFirst)
    expect(answerBlockReason(q({ kind: 'confirm' }), { value: false }, W)).toBe('')
    // `false` is a real answer, not an absent one — this is the classic bug.
    expect(answerBlockReason(q({ kind: 'approve', recommended: false }), undefined, W)).toBe('')
  })
})

describe('answerPayloadFor', () => {
  it('sends an array for multi', () => {
    const m = q({ kind: 'choose', multi: true })
    expect(answerPayloadFor(m, { multi: ['a', 'b'] })).toEqual({ value: ['a', 'b'] })
  })

  it('marks a written-in value as custom', () => {
    const s = q({ kind: 'choose', options: ['a'] })
    expect(answerPayloadFor(s, { customOn: true, custom: ' other ' })).toEqual({
      value: 'other',
      custom: true,
    })
  })

  it('trims free text and preserves booleans exactly', () => {
    expect(answerPayloadFor(q({ kind: 'clarify' }), { value: '  hi  ' })).toEqual({ value: 'hi' })
    expect(answerPayloadFor(q({ kind: 'confirm' }), { value: false })).toEqual({ value: false })
  })
})

describe('displayValue', () => {
  const W2 = { yes: 'Yes', no: 'No' }
  it('renders each answer shape as one short phrase', () => {
    expect(displayValue(true, W2)).toBe('Yes')
    expect(displayValue(false, W2)).toBe('No')
    expect(displayValue(['a', 'b'], W2)).toBe('a, b')
    expect(displayValue({ value: 'other', custom: true }, W2)).toBe('other')
    expect(displayValue('incremental', W2)).toBe('incremental')
  })
})
