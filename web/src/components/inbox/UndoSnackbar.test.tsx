import { describe, it, expect, afterEach } from 'vitest'
import { render, screen, cleanup } from '@testing-library/react'
import { UndoSnackbar } from './UndoSnackbar'
import type { Pending } from '@/lib/undo-queue'

afterEach(cleanup)

const pending: Pending[] = [
  {
    qid: 'q-1',
    payload: { value: true },
    decision: 'Decision: Yes',
    detail: 'demo-1 resumed',
    blocking: true,
    deadline: Date.now() + 30_000,
  },
]

describe('UndoSnackbar', () => {
  it('announces the decision in a polite live region, not the ticking countdown', () => {
    render(
      <UndoSnackbar
        pending={pending}
        now={Date.now()}
        labels={{ undo: 'Undo', seconds: 's' }}
        onUndo={() => {}}
      />,
    )
    const live = screen.getByRole('status')
    expect(live.getAttribute('aria-live')).toBe('polite')
    expect(live.textContent).toContain('Decision: Yes')
    expect(live.textContent).not.toMatch(/\d+s/)
    expect(screen.getByText(/\ds/).getAttribute('aria-hidden')).toBe('true')
  })
})
