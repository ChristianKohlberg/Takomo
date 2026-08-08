// The React side of the trailing-undo queue: timers, commit-on-close, and the
// two flush paths that must never drop a write.
//
// The pure state transitions live in lib/undo-queue.ts and are tested there.
// This owns only the effects.
import { useCallback, useEffect, useRef, useState } from 'react'
import { UNDO_SECONDS, applyPending, due, type Pending } from '@/lib/undo-queue'
import type { AnswerPayload, Question } from '@/lib/questions'

export interface UndoQueueOptions {
  /** Writes the answer for real. Called when a window closes. */
  commit: (p: Pending) => Promise<unknown>
  /** Refetch after a commit, so the list reflects the server. */
  refresh: () => void
  onError: (e: unknown) => void
}

export function useUndoQueue({ commit, refresh, onError }: UndoQueueOptions) {
  const [pending, setPending] = useState<Pending[]>([])
  const [now, setNow] = useState(() => Date.now())

  // Effects read the live set through a ref: the unload handler is installed
  // once, and a stale closure there would silently drop pending writes.
  const pendingRef = useRef<Pending[]>([])
  // Kept in step with state, except where enqueue/undo advance it early (see there).
  if (pending !== pendingRef.current) pendingRef.current = pending
  const commitRef = useRef(commit)
  commitRef.current = commit

  /**
   * Queue an answer. One window per question — a second press is a no-op.
   *
   * The ref is advanced SYNCHRONOUSLY, not just through `setPending`. A caller
   * answers and then folds the queue into its list in the same tick; reading the
   * ref a render later would miss this item, and the question would stay in Open
   * with its own snackbar already counting down — visibly answered and visibly
   * not gone.
   */
  const enqueue = useCallback(
    (q: Question, payload: AnswerPayload, decision: string, detail: string) => {
      if (pendingRef.current.some((p) => p.qid === q.id)) return
      const p: Pending = {
        qid: q.id,
        payload,
        decision,
        detail,
        blocking: q.mode !== 'advisory',
        deadline: Date.now() + UNDO_SECONDS * 1000,
      }
      pendingRef.current = [...pendingRef.current, p]
      setPending(pendingRef.current)
    },
    [],
  )

  /**
   * Cancel a window and RETURN what it displaced, so the caller can put it back.
   *
   * Returning it is the point: a caller that instead looked the snapshot up
   * afterwards would find nothing — `setState` updaters run lazily, by which
   * time this has already dropped the entry. That reads as "Undo worked on the
   * server but the list still shows it answered".
   */
  const undo = useCallback((qid: string): Pending | undefined => {
    const p = pendingRef.current.find((x) => x.qid === qid)
    pendingRef.current = pendingRef.current.filter((x) => x.qid !== qid)
    setPending(pendingRef.current)
    return p
  }, [])

  /** Fold the pending set into a freshly-loaded list. Call on EVERY reload. */
  const apply = useCallback((questions: Question[], actor: string) => {
    const out = applyPending(questions, pendingRef.current, actor)
    // Keep the re-captured snapshots: Undo must restore what the server now says.
    if (out.pending !== pendingRef.current) setPending(out.pending)
    return out.questions
  }, [])

  // One ticker for every window — they share a second, they do not each own a timer.
  useEffect(() => {
    if (!pending.length) return
    const id = window.setInterval(() => setNow(Date.now()), 250)
    return () => window.clearInterval(id)
  }, [pending.length])

  // Commit whatever has run out. Removed from the set BEFORE the request so a
  // slow write cannot be committed twice by the next tick.
  useEffect(() => {
    const ripe = due(pending, now)
    if (!ripe.length) return
    setPending((cur) => cur.filter((p) => !ripe.some((r) => r.qid === p.qid)))
    void (async () => {
      for (const p of ripe) {
        try {
          await commitRef.current(p)
        } catch (e) {
          onError(e)
        }
      }
      refresh()
    })()
  }, [pending, now, refresh, onError])

  /**
   * Write everything still pending, immediately. Used when signing out — the
   * token is about to be dropped, and a pending answer written after that is a
   * 401 rather than a decision.
   */
  const flushAll = useCallback(async () => {
    const all = pendingRef.current
    setPending([])
    for (const p of all) {
      try {
        await commitRef.current(p)
      } catch {
        // Signing out: nothing left to report to.
      }
    }
  }, [])

  // Closing the tab with a window still open must not discard the decision.
  // `keepalive` is what lets the request outlive the page.
  useEffect(() => {
    function onUnload() {
      for (const p of pendingRef.current) void commitRef.current(p)
    }
    window.addEventListener('beforeunload', onUnload)
    return () => window.removeEventListener('beforeunload', onUnload)
  }, [])

  return { pending, now, enqueue, undo, apply, flushAll }
}
