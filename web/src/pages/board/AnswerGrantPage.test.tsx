// The `#a=` page renders its question body as MARKDOWN.
//
// This is the surface that once missed the SPA-wide markdown rendering, so an
// outside expert saw `## Frage` and `| Option | Risiko |` as literal source
// while every internal reader saw them rendered. That reader has the least
// context of anyone: a `tka_` grant shows one question and nothing else.
//
// The Rust suite used to assert this by finding `renderAnswerPage` in the served
// bytes and checking for an `mdNode` call inside it. A bundled page cannot be
// asserted that way — and its own comment said "There is no JS test lane here".
// There is now, and this is it.
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, screen, cleanup, waitFor } from '@testing-library/react'
import { AnswerGrantPage } from './AnswerGrantPage'

vi.mock('@/lib/grants', () => ({
  answerGrantSelf: vi.fn(async () => ({
    expires_at: '2026-08-15T12:00:00Z',
    question: {
      id: 'q-1',
      project: 'demo',
      ticket: 'demo-1',
      kind: 'confirm',
      mode: 'blocking',
      status: 'open',
      title: 'Drop the table?',
      body: '## Frage\n\nSee `billing_v1`, and **note** the risk.\n\n| Option | Risiko |\n|---|---|\n| drop | hoch |',
      options: [],
      option_notes: [],
      multi: false,
      recommended_multi: [],
      expertise: [],
      created_at: '2026-08-08T00:00:00Z',
    },
    ticket: { id: 'demo-1', title: 'Migrate off billing_v1' },
  })),
  submitGrantAnswer: vi.fn(async () => ({})),
}))

const LABELS = {
  yes: 'Yes',
  no: 'No',
  writeOwn: 'own',
  ownPlaceholder: 'own…',
  textPlaceholder: 'answer…',
  recommends: 'Recommends',
  submit: 'Submit',
  typeFirst: 'Type first',
  sendFirst: 'Pick first',
  ticketCtx: 'Ticket context',
  validUntil: 'Valid until',
  thanks: 'Thanks',
  spent: 'Already used',
  expired: 'Expired',
}

afterEach(cleanup)

describe('the #a= answer page', () => {
  it('renders the question body as markdown, not as source', async () => {
    const { container } = render(<AnswerGrantPage token="tka_x" lang="en" labels={LABELS} />)
    await waitFor(() => expect(screen.getByText('Drop the table?')).toBeTruthy())

    // Headings, emphasis, inline code and tables become ELEMENTS. `##` renders
    // as h4, not h3: the renderer starts at h3 because these bodies sit inside
    // panels that already own h1/h2.
    expect(container.querySelector('.md h4')?.textContent).toBe('Frage')
    expect(container.querySelector('.md b')?.textContent).toBe('note')
    expect(container.querySelector('.md code')?.textContent).toBe('billing_v1')
    expect(container.querySelectorAll('.md table tbody tr')).toHaveLength(1)

    // …and their source spelling is nowhere in the rendered text.
    const text = container.textContent ?? ''
    expect(text).not.toContain('## Frage')
    expect(text).not.toContain('| Option | Risiko |')
    expect(text).not.toContain('**note**')
  })

  it('shows the ticket it belongs to — the only context this reader gets', async () => {
    render(<AnswerGrantPage token="tka_x" lang="en" labels={LABELS} />)
    await waitFor(() => expect(screen.getByText('Ticket context')).toBeTruthy())
    expect(screen.getByText('Migrate off billing_v1')).toBeTruthy()
  })

  it('pre-arms nothing when there is no recommendation', async () => {
    render(<AnswerGrantPage token="tka_x" lang="en" labels={LABELS} />)
    await waitFor(() => expect(screen.getByText('Drop the table?')).toBeTruthy())
    // No `recommended`, so the reader must actually choose before submitting.
    expect(screen.getByText('Pick first')).toBeTruthy()
  })
})
