// A check says which part of the plan it verifies, and the card is where that
// link is either useful or an opaque id.
//
// Two rules worth pinning: the section's NAME is resolved from the plan rather
// than stored on the check, so a rename on the map shows here at once; and a
// check whose section has been pruned still shows the link it has, because
// losing what was verified is worse than showing a dangling id.
import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'

import { CheckCard, type CheckCardLabels } from './CheckCard'
import type { CaseCounts, Check } from '@/lib/verification'

const COUNTS: CaseCounts = {
  total: 0,
  approved: 0,
  verified: 0,
  stale: 0,
  failed: 0,
  unreachable: 0,
  blocked: 0,
  never: 0,
}

const LABELS: CheckCardLabels = {
  stateFailed: 'failed',
  stateStale: 'stale',
  stateNever: 'never run',
  stateBlocked: 'blocked',
  stateUnreachable: 'unreachable',
  stateVerified: 'verified',
  stateApproved: 'approved',
  stateNone: 'no cases',
  orphanGlobs: 'matches nothing',
  showCases: 'Show cases',
  hideCases: 'Hide cases',
  noCases: 'No cases',
  approve: 'Approve',
  markPass: 'Pass',
  markFail: 'Fail',
  notePlaceholder: 'what failed',
  archiveCheck: 'Archive',
  verifiesSection: 'Verifies',
  openOnMap: 'Show on the map',
}

function check(over: Partial<Check> = {}): Check {
  return {
    id: 'chk-1',
    project: 'tp',
    title: 'v1 stays v1',
    body: '',
    precondition: '',
    layer: 'api',
    severity: 'blocking',
    epic: null,
    initiative: null,
    node: null,
    globs: [],
    orphan_globs: [],
    environments: [],
    environment_cases: [],
    cases: COUNTS,
    cost_agent_minutes: null,
    cost_human_minutes: null,
    archived_at: null,
    version: 1,
    ...over,
  }
}

function view(props: Partial<Parameters<typeof CheckCard>[0]> = {}) {
  return render(
    <CheckCard
      check={check()}
      cases={undefined}
      loadingCases={false}
      canWrite={false}
      canApprove={false}
      labels={LABELS}
      onToggleCases={vi.fn()}
      onVerdict={vi.fn()}
      onArchive={vi.fn()}
      {...props}
    />,
  )
}

describe('CheckCard', () => {
  it('says nothing about the plan when the check names no section', () => {
    view()
    expect(screen.queryByText('Verifies')).toBeNull()
  })

  it('shows the section by name, resolved rather than stored', () => {
    view({ check: check({ node: 'mn-7' }), nodeTitle: 'version pinning' })
    expect(screen.getByText('Verifies')).toBeTruthy()
    expect(screen.getByText('version pinning')).toBeTruthy()
    expect(screen.queryByText('mn-7')).toBeNull()
  })

  it('keeps the link when the section is gone, rather than hiding it', () => {
    // A pruned node leaves the check readable — deleting a map is ordinary,
    // losing what was verified is not.
    view({ check: check({ node: 'mn-gone' }) })
    expect(screen.getByText('mn-gone')).toBeTruthy()
  })

  it('opens the section on the map, and is inert text when it cannot', () => {
    const onOpenNode = vi.fn()
    const { unmount } = view({
      check: check({ node: 'mn-7' }),
      nodeTitle: 'version pinning',
      onOpenNode,
    })
    fireEvent.click(screen.getByRole('button', { name: 'version pinning' }))
    expect(onOpenNode).toHaveBeenCalledTimes(1)
    unmount()

    view({ check: check({ node: 'mn-7' }), nodeTitle: 'version pinning' })
    expect(screen.queryByRole('button', { name: 'version pinning' })).toBeNull()
    expect(screen.getByText('version pinning')).toBeTruthy()
  })
})
