// Trailing undo: an answer takes effect optimistically at once — the item is
// marked done and drops out of Open — while a 30s window runs in the background.
// Only Undo brings it back. The write itself happens when the window closes.
//
// Every answer gets its OWN window. Working through the inbox quickly leaves
// several running side by side, each with its own countdown and its own Undo,
// and answering a new question neither commits nor discards the ones already
// pending.
//
// The invariant that is easy to lose (and that the page this replaces documents
// as a bug it hit): a pending answer exists only in this tab until its window
// closes, so every reload of the question list comes back with those questions
// still `open` on the server. Left alone that would undo their optimistic
// completion — the item pops back into Open and its count while its own snackbar
// is still counting down, and since it is still pending, pressing Answer again
// does nothing. So EVERY reload re-applies the pending set. That is why
// `applyPending` is a pure function over a freshly-fetched list rather than a
// one-shot mutation at submit time.
import type { AnswerPayload, Question } from './questions'

export const UNDO_SECONDS = 30

/** What a pending answer displaced, so Undo can put it back. */
export interface Snapshot {
  status: Question['status']
  answer: Question['answer']
  awaiting: Question['awaiting']
  resolved_to: Question['resolved_to']
  answered_by: Question['answered_by']
}

export interface Pending {
  qid: string
  payload: AnswerPayload
  /** One-line summary of the decision, for the snackbar. */
  decision: string
  /** What it did: resumed a ticket, or was merely recorded. */
  detail: string
  /** Blocking questions resume their ticket; advisory ones do not. */
  blocking: boolean
  /** Epoch ms at which this commits. */
  deadline: number
  /**
   * Re-captured on every apply, so Undo restores what the server currently says
   * rather than a snapshot taken up to 30s earlier.
   */
  prev?: Snapshot | null
}

export function snapshot(q: Question): Snapshot {
  return {
    status: q.status,
    answer: q.answer ?? null,
    awaiting: q.awaiting ?? null,
    resolved_to: q.resolved_to ?? null,
    answered_by: q.answered_by ?? null,
  }
}

/**
 * Fold the pending set into a freshly-loaded question list.
 *
 * Returns a new list AND the pending set with `prev` re-captured. A pending
 * answer whose question is not in this view (project switched, ticket or tag
 * filter) is left alone — it still commits on time, it just has nothing to
 * paint.
 */
export function applyPending(
  questions: Question[],
  pending: Pending[],
  actor: string,
): { questions: Question[]; pending: Pending[] } {
  if (!pending.length) return { questions, pending }

  const byId = new Map(questions.map((q) => [q.id, q]))
  const nextPending = pending.map((p) => {
    const q = byId.get(p.qid)
    return q ? { ...p, prev: snapshot(q) } : p
  })

  const nextQuestions = questions.map((q) => {
    const p = pending.find((x) => x.qid === q.id)
    if (!p) return q
    return {
      ...q,
      status: 'answered' as const,
      answer: p.payload.custom
        ? { value: p.payload.value, custom: true }
        : { value: p.payload.value },
      answered_by: actor,
    }
  })

  return { questions: nextQuestions, pending: nextPending }
}

/** Put back what a pending answer displaced. */
export function undoInto(questions: Question[], p: Pending): Question[] {
  if (!p.prev) return questions
  return questions.map((q) => (q.id === p.qid ? { ...q, ...p.prev } : q))
}

/** Seconds left, floored at 0 — what the snackbar counts down. */
export function secondsLeft(p: Pending, now: number): number {
  return Math.max(0, Math.ceil((p.deadline - now) / 1000))
}

/** The ones whose window has closed and which must now be written. */
export function due(pending: Pending[], now: number): Pending[] {
  return pending.filter((p) => p.deadline <= now)
}
