// What a section says about itself: where it stands, who did what to it, and
// which of the two views owns its title.
import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { SectionPanel } from './SectionPanel'
import type { TraceEntry } from '@/lib/mindmaps'

const labels = {
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
  it('heads the section with its address and its title, at its level', () => {
    panel()
    expect(screen.getByText('2.1')).toBeTruthy()
    expect(screen.getByRole('heading', { level: 3 }).textContent).toBe('Versioning')
  })

  it('reads a top-level section as the plan\'s first heading level', () => {
    panel({ depth: 0 })
    expect(screen.getByRole('heading', { level: 2 })).toBeTruthy()
  })

  it('never offers to edit the title, because that caret lives on the map', () => {
    panel()
    expect(screen.queryByRole('textbox')).toBeNull()
    expect(screen.getByText('⌖ Show it on the map')).toBeTruthy()
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
