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
  it('reads top down: overall progress, then sections, then behaviors in plain words', async () => {
    show()
    expect(await screen.findByText('1 of 2 behaviors verified')).toBeTruthy()
    expect(screen.getByText(/No test run reported yet\./)).toBeTruthy()
    const sections = screen.getAllByRole('button', { name: /^Show or hide/ })
    // The section with a failure comes first; the fully verified one starts folded.
    expect(sections.map((b) => b.getAttribute('aria-label'))).toEqual(['Show or hide Sharing', 'Show or hide Saving'])
    expect(sections[0]!.getAttribute('aria-expanded')).toBe('true')
    expect(sections[1]!.getAttribute('aria-expanded')).toBe('false')
    expect(screen.getByText('0/1 verified')).toBeTruthy()
    const failing = screen.getByText('Share link opens read-only').closest('button')!
    expect(within(failing).getByText('No test linked yet')).toBeTruthy()
    expect(screen.queryByText('Failed save keeps edits')).toBeNull()

    fireEvent.click(sections[1]!)
    const verified = screen.getByText('Failed save keeps edits').closest('button')!
    expect(within(verified).getByText(/^Passed \d+s ago$/)).toBeTruthy()
    // No test keys on the list: they wait until a behavior is opened.
    expect(screen.queryByText(/playwright:/)).toBeNull()
  })

  it('filters with the status chips and the search in the ribbon', async () => {
    show()
    await screen.findByText('Share link opens read-only')
    const chips = within(screen.getByRole('group', { name: 'Filter by status' }))
    expect(chips.getByRole('button', { name: /All\s*2/ })).toBeTruthy()
    fireEvent.click(chips.getByRole('button', { name: /Verified\s*1/ }))
    expect(screen.queryByText('Share link opens read-only')).toBeNull()
    // A filter opens the folded section it looks into.
    expect(screen.getByText('Failed save keeps edits')).toBeTruthy()
    fireEvent.click(chips.getByRole('button', { name: /All\s*2/ }))
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search behaviors' }), { target: { value: 'nothing like it' } })
    expect(screen.getByText('No behaviors match these filters.')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }))
    expect(screen.getByText('Share link opens read-only')).toBeTruthy()
    expect(screen.queryByRole('button', { name: /Refresh/ })).toBeNull()
  })

  it('narrows to the selected section and uses its counts', async () => {
    show('/projects/demo/specification?view=tests&section=mn-share')
    expect(await screen.findByText('Share link opens read-only')).toBeTruthy()
    expect(screen.queryByText('Failed save keeps edits')).toBeNull()
    expect(screen.getByText('0 of 1 behaviors verified')).toBeTruthy()
    expect(screen.queryByText(/reported tests belong to no behavior/)).toBeNull()
  })

  it('opens a behavior in place when its row is chosen, and closes it again', async () => {
    show()
    fireEvent.click(await screen.findByText('Share link opens read-only'))
    expect(mocks.openBehavior).toHaveBeenCalledWith('bhv-share')
  })

  it('shows an opened behavior as prose and the tests that show it, technical details on request', async () => {
    const detail: BehaviorDetail = {
      ...behavior({ status: 'failing' }),
      tests: ['cargo:api::save_conflict', 'playwright:editor.spec.ts › keeps edits'],
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
    const shownBy = await screen.findByRole('region', { name: 'Shown by' })
    expect(screen.getByText('When saving fails, edits stay.')).toBeTruthy()
    expect(within(shownBy).getByText('keeps edits')).toBeTruthy()
    expect(within(shownBy).getByText(/^Browser · failed \d+s ago$/)).toBeTruthy()
    expect(within(shownBy).getByText('Timeout after 5s')).toBeTruthy()
    expect(within(shownBy).getByText('save conflict')).toBeTruthy()
    expect(within(shownBy).getByText('Unit · never reported')).toBeTruthy()
    // Raw keys, the run history and the edit form are one click away, not on screen.
    expect(screen.queryByText('cargo:api::save_conflict')).toBeNull()
    expect(screen.queryByText('nightly')).toBeNull()
    expect(screen.queryByLabelText('Title')).toBeNull()

    fireEvent.click(screen.getByRole('button', { name: 'Technical details' }))
    expect(screen.getByText('cargo:api::save_conflict')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'History' }))
    expect(screen.getByText('nightly')).toBeTruthy()

    fireEvent.click(within(shownBy).getByRole('button', { name: 'Unlink save conflict' }))
    await waitFor(() =>
      expect(mocks.patchBehavior).toHaveBeenCalledWith('token', 'bhv-save', { remove_tests: ['cargo:api::save_conflict'] }),
    )
    expect(mocks.refreshVerification).toHaveBeenCalled()
  })

  it('edits title, text and section together behind one Save', async () => {
    mocks.getBehavior.mockResolvedValue({ ...behavior(), test_results: [], history: [] })
    mocks.patchBehavior.mockResolvedValue(behavior())
    show('/projects/demo/specification?view=tests&behavior=bhv-save')
    fireEvent.click(await screen.findByRole('button', { name: 'Edit' }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Failed save keeps every edit' } })
    fireEvent.change(screen.getByLabelText('Section of the specification'), { target: { value: 'mn-share' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() =>
      expect(mocks.patchBehavior).toHaveBeenCalledWith('token', 'bhv-save', {
        title: 'Failed save keeps every edit',
        statement: 'When saving fails, edits stay.',
        section: 'mn-share',
      }),
    )
    await waitFor(() => expect(screen.queryByLabelText('Title')).toBeNull())
  })

  it('links a test to an opened behavior', async () => {
    mocks.getBehavior.mockResolvedValue({ ...behavior(), test_results: [], history: [] })
    mocks.patchBehavior.mockResolvedValue(behavior())
    show('/projects/demo/specification?view=tests&behavior=bhv-save')
    fireEvent.click(await screen.findByRole('button', { name: 'Link a test' }))
    fireEvent.change(screen.getByLabelText('Test key, as CI reports it'), { target: { value: 'cargo:api::retry' } })
    fireEvent.click(screen.getByRole('button', { name: 'Link' }))
    await waitFor(() =>
      expect(mocks.patchBehavior).toHaveBeenCalledWith('token', 'bhv-save', { add_tests: ['cargo:api::retry'] }),
    )
  })

  it('creates a behavior with its section and linked tests', async () => {
    mocks.createBehavior.mockResolvedValue(behavior({ id: 'bhv-new' }))
    show('/projects/demo/specification?view=tests&section=mn-save')
    await screen.findByText('Failed save keeps edits')
    fireEvent.click(screen.getByRole('button', { name: 'New behavior' }))
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

  it('keeps reported but unlinked tests folded, and links one to a behavior', async () => {
    mocks.patchBehavior.mockResolvedValue(behavior())
    show()
    const toggle = await screen.findByRole('button', { name: /1 reported tests belong to no behavior/ })
    expect(screen.queryByRole('combobox', { name: 'Link cargo:api::orphan to a behavior' })).toBeNull()
    fireEvent.click(toggle)
    fireEvent.change(screen.getByRole('combobox', { name: 'Link cargo:api::orphan to a behavior' }), {
      target: { value: 'bhv-share' },
    })
    await waitFor(() =>
      expect(mocks.patchBehavior).toHaveBeenCalledWith('token', 'bhv-share', { add_tests: ['cargo:api::orphan'] }),
    )
  })

  it('explains the loop when there is nothing yet, and hides writes without scope', async () => {
    mocks.scopes = ['read']
    mocks.listBehaviors.mockResolvedValue({ items: [], total: 0, limit: 500 })
    show()
    expect(await screen.findByText(/No behaviors yet\. Describe what the software must do/)).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'New behavior' })).toBeNull()
  })

  it('offers no writes in an archived project, which would refuse them', async () => {
    mocks.archived = true
    mocks.listBehaviors.mockResolvedValue({ items: [], total: 0, limit: 500 })
    show()
    expect(await screen.findByText(/No behaviors yet/)).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'New behavior' })).toBeNull()
  })
})
