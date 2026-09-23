import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Behavior, BehaviorDetail, VerificationSummary } from '@/lib/behaviors'
import { TestsView } from './App'

const mocks = vi.hoisted(() => ({
  listBehaviors: vi.fn(),
  getBehavior: vi.fn(),
  createBehavior: vi.fn(),
  patchBehavior: vi.fn(),
  deleteBehavior: vi.fn(),
  refreshVerification: vi.fn(),
  openBehavior: vi.fn(),
  onError: vi.fn(),
  scopes: ['read', 'write'] as string[],
  archived: false,
  verification: null as VerificationSummary | null,
}))
vi.mock('@/lib/behaviors', async (original) => ({
  ...(await original<typeof import('@/lib/behaviors')>()),
  listBehaviors: mocks.listBehaviors,
  getBehavior: mocks.getBehavior,
  createBehavior: mocks.createBehavior,
  patchBehavior: mocks.patchBehavior,
  deleteBehavior: mocks.deleteBehavior,
}))
vi.mock('@/hooks/useProjectUpdates', async (original) => ({
  ...(await original<typeof import('@/hooks/useProjectUpdates')>()),
  useProjectUpdates: vi.fn(),
}))
vi.mock('../specification/context', () => ({
  useSpecification: () => ({
    token: 'token',
    lang: 'en',
    project: 'demo',
    scopes: mocks.scopes,
    projects: [{ id: 'demo', name: 'Demo', archived: mocks.archived }],
    nodes: [
      { id: 'mn-save', parent: null, order: 'a', title: 'Saving', position: 0 },
      { id: 'mn-share', parent: null, order: 'b', title: 'Sharing', position: 1 },
    ],
    verification: mocks.verification,
    refreshVerification: mocks.refreshVerification,
    openBehavior: mocks.openBehavior,
    onError: mocks.onError,
  }),
}))

const now = new Date().toISOString()
function behavior(over: Partial<Behavior> = {}): Behavior {
  return {
    id: 'bhv-save',
    project: 'demo',
    section: 'mn-save',
    title: 'Failed save keeps edits',
    statement: 'When saving fails, edits stay.',
    tests: ['playwright:editor.spec.ts › keeps edits'],
    status: 'verified',
    last_result: { test: 'playwright:editor.spec.ts › keeps edits', outcome: 'pass', at: now, commit: 'a1b2c3d4e5', run: 'vrn-1' },
    created_by: 'agent:ci',
    created_at: now,
    updated_at: now,
    ...over,
  }
}
const counts = (over = {}) => ({ total: 0, verified: 0, failing: 0, stale: 0, untested: 0, ...over })

function show(url = '/projects/demo/specification?view=tests') {
  return render(
    <MemoryRouter initialEntries={[url]}>
      <TestsView />
    </MemoryRouter>,
  )
}

beforeEach(() => {
  vi.clearAllMocks()
  mocks.scopes = ['read', 'write']
  mocks.archived = false
  mocks.verification = {
    fresh_days: 14,
    summary: counts({ total: 2, verified: 1, failing: 1 }),
    sections: { 'mn-save': counts({ total: 1, verified: 1 }), 'mn-share': counts({ total: 1, failing: 1 }) },
    unsectioned: 0,
    unlinked_tests: { items: [{ test: 'cargo:api::orphan', outcome: 'pass', at: now, commit: null }], total: 1, limit: 50 },
    latest_run: null,
  }
  mocks.listBehaviors.mockResolvedValue({
    items: [
      behavior(),
      behavior({ id: 'bhv-share', section: 'mn-share', title: 'Share link opens read-only', status: 'failing', tests: [], last_result: null }),
    ],
    total: 2,
    limit: 500,
  })
  mocks.refreshVerification.mockResolvedValue(null)
})
afterEach(cleanup)

describe('TestsView', () => {
  it('lists behaviors with status, linked tests and the latest result', async () => {
    show()
    const row = (await screen.findByRole('heading', { name: 'Failed save keeps edits' })).closest('button')!
    expect(within(row).getByText('Verified')).toBeTruthy()
    expect(within(row).getByText(/1 test · a1b2c3d · \d+s · Saving/)).toBeTruthy()
    expect(screen.getByRole('heading', { name: 'Share link opens read-only' }).closest('button')!.textContent).toContain('No linked tests')
    expect(screen.getByRole('button', { name: 'Failing · 1' })).toBeTruthy()
    expect(screen.getByText('Verified means a linked test passed in the last 14 days.')).toBeTruthy()
  })

  it('filters by status and by text', async () => {
    show()
    await screen.findByRole('heading', { name: 'Failed save keeps edits' })
    fireEvent.click(screen.getByRole('button', { name: 'Failing · 1' }))
    expect(screen.queryByRole('heading', { name: 'Failed save keeps edits' })).toBeNull()
    expect(screen.getByRole('heading', { name: 'Share link opens read-only' })).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }))
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search behaviors' }), { target: { value: 'editor.spec' } })
    expect(screen.getByRole('heading', { name: 'Failed save keeps edits' })).toBeTruthy()
    expect(screen.queryByRole('heading', { name: 'Share link opens read-only' })).toBeNull()
  })

  it('narrows to the selected section and uses its counts', async () => {
    show('/projects/demo/specification?view=tests&section=mn-share')
    expect(await screen.findByText('Share link opens read-only')).toBeTruthy()
    expect(screen.queryByText('Failed save keeps edits')).toBeNull()
    expect(screen.getByRole('button', { name: 'Verified · 0' })).toBeTruthy()
    expect(screen.queryByText('Reported tests without a behavior · 1')).toBeNull()
  })

  it('opens a behavior when its row is chosen', async () => {
    show()
    fireEvent.click(await screen.findByRole('heading', { name: 'Failed save keeps edits' }))
    expect(mocks.openBehavior).toHaveBeenCalledWith('bhv-save')
  })

  it('shows the selected behavior with its tests and history, and saves edits', async () => {
    const detail: BehaviorDetail = {
      ...behavior(),
      test_results: [
        { test: 'playwright:editor.spec.ts › keeps edits', latest: { outcome: 'fail', detail: 'Timeout after 5s', at: now, commit: 'ffee001', run: 'vrn-2', actor: 'agent:ci' } },
        { test: 'cargo:api::save_conflict', latest: null },
      ],
      history: [
        { test: 'playwright:editor.spec.ts › keeps edits', outcome: 'fail', detail: 'Timeout after 5s', at: now, commit: 'ffee001', run: 'vrn-2', note: 'nightly', actor: 'agent:ci' },
      ],
    }
    mocks.getBehavior.mockResolvedValue(detail)
    mocks.patchBehavior.mockResolvedValue(behavior())
    show('/projects/demo/specification?view=tests&behavior=bhv-save')
    const title = await screen.findByLabelText('Title')
    expect(screen.getByText('not reported yet')).toBeTruthy()
    expect(screen.getAllByText('Timeout after 5s')).toHaveLength(2)
    expect(screen.getByText('nightly')).toBeTruthy()

    fireEvent.change(title, { target: { value: 'Failed save keeps every edit' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() =>
      expect(mocks.patchBehavior).toHaveBeenCalledWith('token', 'bhv-save', {
        title: 'Failed save keeps every edit',
        statement: 'When saving fails, edits stay.',
      }),
    )

    fireEvent.click(screen.getByRole('button', { name: 'Remove cargo:api::save_conflict' }))
    await waitFor(() =>
      expect(mocks.patchBehavior).toHaveBeenCalledWith('token', 'bhv-save', {
        tests: ['playwright:editor.spec.ts › keeps edits'],
      }),
    )
    expect(mocks.refreshVerification).toHaveBeenCalled()
  })

  it('creates a behavior with its section and linked tests', async () => {
    mocks.createBehavior.mockResolvedValue(behavior({ id: 'bhv-new' }))
    show('/projects/demo/specification?view=tests&section=mn-save')
    await screen.findByText('Failed save keeps edits')
    fireEvent.click(screen.getByRole('button', { name: '+ New behavior' }))
    const dialog = await screen.findByRole('dialog')
    fireEvent.click(within(dialog).getByRole('button', { name: 'Create' }))
    expect(await within(dialog).findByText('A behavior needs a title.')).toBeTruthy()
    fireEvent.change(within(dialog).getByLabelText('Title'), { target: { value: 'Retry saves edits' } })
    fireEvent.change(within(dialog).getByLabelText('Linked tests'), {
      target: { value: 'cargo:api::retry\n\n cargo:api::retry \nplaywright:retry' },
    })
    fireEvent.click(within(dialog).getByRole('button', { name: 'Create' }))
    await waitFor(() =>
      expect(mocks.createBehavior).toHaveBeenCalledWith('token', 'demo', {
        title: 'Retry saves edits',
        statement: '',
        section: 'mn-save',
        tests: ['cargo:api::retry', 'playwright:retry'],
      }),
    )
    await waitFor(() => expect(mocks.openBehavior).toHaveBeenCalledWith('bhv-new'))
  })

  it('links a reported but unlinked test to a behavior', async () => {
    mocks.patchBehavior.mockResolvedValue(behavior())
    show()
    await screen.findByText('Reported tests without a behavior · 1')
    fireEvent.change(screen.getByRole('combobox', { name: 'Link cargo:api::orphan to a behavior' }), {
      target: { value: 'bhv-share' },
    })
    await waitFor(() =>
      expect(mocks.patchBehavior).toHaveBeenCalledWith('token', 'bhv-share', { tests: ['cargo:api::orphan'] }),
    )
  })

  it('explains the loop when there is nothing yet, and hides writes without scope', async () => {
    mocks.scopes = ['read']
    mocks.listBehaviors.mockResolvedValue({ items: [], total: 0, limit: 500 })
    show()
    expect(await screen.findByText(/No behaviors yet\. Describe what the software must do/)).toBeTruthy()
    expect(screen.queryByRole('button', { name: '+ New behavior' })).toBeNull()
  })

  it('offers no writes in an archived project, which would refuse them', async () => {
    mocks.archived = true
    mocks.listBehaviors.mockResolvedValue({ items: [], total: 0, limit: 500 })
    show()
    expect(await screen.findByText(/No behaviors yet/)).toBeTruthy()
    expect(screen.queryByRole('button', { name: '+ New behavior' })).toBeNull()
  })
})
