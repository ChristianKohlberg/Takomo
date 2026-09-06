// What a section says about itself: where it stands, who did what to it, and
// which of the two views owns its title.
import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { SectionPanel } from './SectionPanel'
import type { TraceEntry } from '@/lib/mindmaps'

const labels = {
  renameSection: 'Rename section',
  untitled: 'Untitled section',
  standingConfirmed: 'agreed',
  standingChanged: 'changed since',
  standingUnseen: 'unread',
  review: 'I have read this',
  reviewHint: 'Records that you agree',
  showOnMap: 'Show it on the map',
  history: 'History',
  hideHistory: 'Hide history',
  historyEmpty: 'Nothing recorded for this section yet.',
  historyMore: '{n} older entries',
  proposals: 'Proposals',
  hideProposals: 'Hide proposals',
  pendingBadge: '{n} waiting',
  needWrite: "This needs a token with the 'write' scope.",
  kinds: {
    authored: 'written',
    renamed: 'renamed',
    edited: 'edited',
    moved: 'moved',
    pruned: 'removed',
    reviewed: 'read',
    proposed: 'proposed',
    accepted: 'accepted',
    rejected: 'rejected',
  },
}

function entry(over: Partial<TraceEntry> = {}): TraceEntry {
  return {
    id: 'tr-1',
    node: 'mn-1',
    kind: 'edited',
    actor: 'agent:fleet-3',
    user: null,
    note: null,
    text: null,
    at: new Date().toISOString(),
    ...over,
  }
}

function panel(over: Partial<Parameters<typeof SectionPanel>[0]> = {}) {
  return render(
    <SectionPanel
      number="2.1"
      depth={1}
      title="Versioning"
      standing="unseen"
      entries={[]}
      historyOpen={false}
      onToggleHistory={() => {}}
      onReview={() => {}}
      onShowOnMap={() => {}}
      canWrite
      labels={labels}
      {...over}
    >
      <p>prose</p>
    </SectionPanel>,
  )
}

describe('SectionPanel', () => {
  it('keeps pending work visible while its compact action menu opens and closes', () => {
    panel({ pending: 2 })
    const menu = screen.getByRole('button', { name: 'Section actions' })
    const pending = screen.getByText('◆ 2 waiting')
    expect(menu.getAttribute('aria-expanded')).toBe('false')
    fireEvent.click(menu)
    expect(menu.getAttribute('aria-expanded')).toBe('true')
    fireEvent.pointerDown(screen.getByText('⌖ Show it on the map'))
    expect(menu.getAttribute('aria-expanded')).toBe('true')
    fireEvent.keyDown(screen.getByText('⌖ Show it on the map'), { key: 'Escape' })
    expect(menu.getAttribute('aria-expanded')).toBe('false')
    expect(document.activeElement).toBe(menu)
    expect(pending.isConnected).toBe(true)
    fireEvent.click(menu)
    fireEvent.pointerDown(document.body)
    expect(menu.getAttribute('aria-expanded')).toBe('false')
  })

  it('closes the open menu on Escape typed elsewhere without pulling focus away from there', () => {
    panel()
    const input = document.createElement('input')
    document.body.append(input)
    try {
      const menu = screen.getByRole('button', { name: 'Section actions' })
      fireEvent.click(menu)
      expect(menu.getAttribute('aria-expanded')).toBe('true')
      input.focus()
      fireEvent.keyDown(input, { key: 'Escape' })
      expect(menu.getAttribute('aria-expanded')).toBe('false')
      expect(document.activeElement).toBe(input)
    } finally {
      input.remove()
    }
  })

  it('heads the section with its address and its title, at its level', () => {
    panel()
    expect(screen.getByText('2.1')).toBeTruthy()
    expect(screen.getByRole('heading', { level: 2 }).textContent).toBe('Versioning')
  })

  it('reads a top-level section as the plan\'s first heading level', () => {
    panel({ depth: 0 })
    expect(screen.getByRole('heading', { level: 1 })).toBeTruthy()
  })

  it('renames the section in place, and still offers the map', async () => {
    // This used to assert the opposite — the heading was read-only because a
    // title is one `Y.Text` and a caret in two layouts is a fight. That holds
    // for a LIVE caret; `EditableText` commits on blur, which is one diff, the
    // same shape as the map's own rename. Writing a plan and naming its parts
    // is one activity, so it happens where you are looking.
    const onTitle = vi.fn()
    panel({ canWrite: true, onTitle })
    // `EditableText` is a `contentEditable` heading, not an input — queried by
    // its label, which is the thing a screen reader announces.
    const box = screen.getByLabelText('Rename section')
    expect(box.getAttribute('contenteditable')).toBe('true')
    box.textContent = 'versioning, decided'
    fireEvent.blur(box)
    expect(onTitle).toHaveBeenCalledWith('versioning, decided')
    // Showing it on the map stays, for when the map is where you want to be.
    expect(screen.getByText('⌖ Show it on the map')).toBeTruthy()
  })

  it('leaves the heading static for a reader', () => {
    panel({ canWrite: false })
    expect(screen.queryByLabelText('Rename section')).toBeNull()
  })

  it('says where the section stands in words', () => {
    panel({ standing: 'changed' })
    expect(screen.getByText('changed since')).toBeTruthy()
  })

  it('records a review, and refuses to offer one to a token that cannot write', () => {
    const onReview = vi.fn()
    const { rerender } = panel({ onReview })
    screen.getByText('✓ I have read this').click()
    expect(onReview).toHaveBeenCalled()

    rerender(
      <SectionPanel
        number="2.1"
        depth={1}
        title="Versioning"
        standing="unseen"
        entries={[]}
        historyOpen={false}
        onToggleHistory={() => {}}
        onReview={onReview}
        onShowOnMap={() => {}}
        canWrite={false}
        labels={labels}
      >
        <p>prose</p>
      </SectionPanel>,
    )
    expect(screen.getByText('✓ I have read this').closest('button')?.disabled).toBe(true)
  })

  it('says a proposal is waiting without anything being opened', () => {
    // The reader has to be able to FIND one. A proposal only visible after
    // somebody opens the right panel is one that gets accepted by whoever
    // happens to look, which is not review.
    panel({ pending: 2, proposalCount: 3 })
    expect(screen.getByText('◆ 2 waiting')).toBeTruthy()
  })

  it('offers no proposal toggle where nothing has been offered', () => {
    panel()
    expect(screen.queryByText('◆ Proposals')).toBeNull()
  })

  it('keeps the toggle after the last one is decided, because a decision is a record', () => {
    panel({ pending: 0, proposalCount: 1, onToggleProposals: () => {} })
    expect(screen.getByText('◆ Proposals')).toBeTruthy()
    expect(screen.queryByText(/waiting/)).toBeNull()
  })

  it('shows the review panel only once it is opened', () => {
    const onToggleProposals = vi.fn()
    const { rerender } = panel({
      pending: 1,
      proposalCount: 1,
      onToggleProposals,
      proposals: <p>the offer</p>,
    })
    expect(screen.queryByText('the offer')).toBeNull()
    screen.getByText('◆ Proposals').click()
    expect(onToggleProposals).toHaveBeenCalled()

    rerender(
      <SectionPanel
        number="2.1"
        depth={1}
        title="Versioning"
        standing="unseen"
        entries={[]}
        historyOpen={false}
        onToggleHistory={() => {}}
        onReview={() => {}}
        onShowOnMap={() => {}}
        canWrite
        pending={1}
        proposalCount={1}
        proposalsOpen
        onToggleProposals={onToggleProposals}
        proposals={<p>the offer</p>}
        labels={labels}
      >
        <p>prose</p>
      </SectionPanel>,
    )
    expect(screen.getByText('the offer')).toBeTruthy()
  })

  it('names the person behind an act where there is one, and the actor otherwise', () => {
    panel({
      historyOpen: true,
      entries: [entry({ id: 'a', user: 'usr-7', kind: 'reviewed' }), entry({ id: 'b' })],
    })
    expect(screen.getByText('usr-7')).toBeTruthy()
    expect(screen.getByText('agent:fleet-3')).toBeTruthy()
    expect(screen.getByText('read')).toBeTruthy()
  })

  it('says how much history it is not showing rather than dropping it silently', () => {
    panel({
      historyOpen: true,
      historyLimit: 2,
      entries: [entry({ id: 'a' }), entry({ id: 'b' }), entry({ id: 'c' })],
    })
    expect(screen.getByText('1 older entries')).toBeTruthy()
  })

  it('says so when a section has no history at all', () => {
    panel({ historyOpen: true })
    expect(screen.getByText('Nothing recorded for this section yet.')).toBeTruthy()
  })
})

describe('SectionPanel depth', () => {
  it('keeps H1–H3 on one left edge and steps H4–H6 in by depth', () => {
    const insetAt = (depth: number) => {
      const { container, unmount } = panel({ depth })
      const inset = (container.querySelector('section') as HTMLElement).style.paddingLeft
      unmount()
      return inset
    }
    expect([0, 1, 2].map(insetAt)).toEqual(['0px', '0px', '0px'])
    const [h4, h5, h6] = [3, 4, 5].map((d) => parseInt(insetAt(d), 10)) as [number, number, number]
    expect(h4).toBeGreaterThan(0)
    expect(h5).toBeGreaterThan(h4)
    expect(h6).toBeGreaterThan(h5)
    expect(insetAt(9)).toBe(insetAt(5))
  })

  it('still marks a deep section with its semantic heading level', () => {
    panel({ depth: 4 })
    expect(screen.getByRole('heading', { level: 5 }).textContent).toBe('Versioning')
  })
})
