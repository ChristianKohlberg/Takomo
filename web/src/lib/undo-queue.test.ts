import { describe, it, expect } from 'vitest'
import { applyPending, due, secondsLeft, snapshot, undoInto, type Pending } from './undo-queue'
import type { Question } from './questions'

function q(over: Partial<Question> = {}): Question {
  return {
    id: 'q-1',
    project: 'demo',
    ticket: 'demo-1',
    kind: 'confirm',
    mode: 'blocking',
    status: 'open',
    title: 'A question',
    options: [],
    option_notes: [],
    multi: false,
    recommended_multi: [],
    expertise: [],
    awaiting: 'human',
    created_at: '2026-08-08T00:00:00Z',
    ...over,
  }
}

function pending(over: Partial<Pending> = {}): Pending {
  return {
    qid: 'q-1',
    payload: { value: true },
    decision: 'Decision: Yes',
    detail: 'demo-1 resumed',
    blocking: true,
    deadline: 1_000_000,
    ...over,
  }
}

describe('applyPending', () => {
  it('optimistically completes a pending question so it leaves the Open folder', () => {
    const { questions } = applyPending([q()], [pending()], 'human:ada')
    expect(questions[0]!.status).toBe('answered')
    expect(questions[0]!.answer).toEqual({ value: true })
    expect(questions[0]!.answered_by).toBe('human:ada')
  })

  it('marks a written-in value as custom', () => {
    const { questions } = applyPending(
      [q()],
      [pending({ payload: { value: 'other', custom: true } })],
      'human:ada',
    )
    expect(questions[0]!.answer).toEqual({ value: 'other', custom: true })
  })

  it('RE-APPLIES on every reload — the invariant', () => {
    // A reload brings the question back `open` from the server. Left alone it
    // would pop back into Open while its own snackbar is still counting down.
    const fresh = [q({ status: 'open', answer: null })]
    const { questions } = applyPending(fresh, [pending()], 'human:ada')
    expect(questions[0]!.status).toBe('answered')
  })

  it('re-captures prev each time, so Undo restores what the server NOW says', () => {
    const first = applyPending([q({ status: 'open' })], [pending()], 'human:ada')
    expect(first.pending[0]!.prev?.status).toBe('open')

    // The server has since moved the question on its own (say, it expired).
    const second = applyPending([q({ status: 'expired' })], first.pending, 'human:ada')
    expect(second.pending[0]!.prev?.status).toBe('expired')
  })

  it('leaves a pending answer alone when its question is not in view', () => {
    // Project switched or a filter applied: nothing to paint, but it still commits.
    const { questions, pending: p } = applyPending([q({ id: 'other' })], [pending()], 'human:ada')
    expect(questions[0]!.status).toBe('open')
    expect(p[0]!.prev).toBeUndefined()
  })

  it('handles several windows side by side without disturbing each other', () => {
    const list = [q({ id: 'a' }), q({ id: 'b' }), q({ id: 'c' })]
    const { questions } = applyPending(
      list,
      [pending({ qid: 'a' }), pending({ qid: 'c', payload: { value: false } })],
      'human:ada',
    )
    expect(questions.map((x) => x.status)).toEqual(['answered', 'open', 'answered'])
    expect(questions[2]!.answer).toEqual({ value: false })
  })

  it('is a no-op with nothing pending', () => {
    const list = [q()]
    expect(applyPending(list, [], 'human:ada').questions).toBe(list)
  })
})

describe('undoInto', () => {
  it('puts back exactly what was displaced', () => {
    const before = q({ status: 'open', awaiting: 'human' })
    const { questions, pending: p } = applyPending([before], [pending()], 'human:ada')
    expect(questions[0]!.status).toBe('answered')

    const restored = undoInto(questions, p[0]!)
    expect(restored[0]!.status).toBe('open')
    expect(restored[0]!.answer).toBeNull()
    expect(restored[0]!.answered_by).toBeNull()
  })

  it('does nothing when there is no snapshot to restore', () => {
    const list = [q()]
    expect(undoInto(list, pending({ prev: null }))).toEqual(list)
  })
})

describe('the countdown', () => {
  it('counts down in whole seconds and floors at zero', () => {
    expect(secondsLeft(pending({ deadline: 30_000 }), 0)).toBe(30)
    expect(secondsLeft(pending({ deadline: 30_000 }), 29_100)).toBe(1)
    expect(secondsLeft(pending({ deadline: 30_000 }), 30_000)).toBe(0)
    expect(secondsLeft(pending({ deadline: 30_000 }), 45_000)).toBe(0)
  })

  it('reports exactly the windows that have closed', () => {
    const a = pending({ qid: 'a', deadline: 100 })
    const b = pending({ qid: 'b', deadline: 500 })
    expect(due([a, b], 99).map((p) => p.qid)).toEqual([])
    expect(due([a, b], 100).map((p) => p.qid)).toEqual(['a'])
    expect(due([a, b], 900).map((p) => p.qid)).toEqual(['a', 'b'])
  })
})

describe('snapshot', () => {
  it('normalizes absent fields to null so a restore is total', () => {
    expect(snapshot(q({ answer: undefined, resolved_to: undefined }))).toEqual({
      status: 'open',
      answer: null,
      awaiting: 'human',
      resolved_to: null,
      answered_by: null,
    })
  })
})
