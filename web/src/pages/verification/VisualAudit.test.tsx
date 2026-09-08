import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { MemoryRouter } from 'react-router'
import { TestsView } from './App'
import type { TestDefinition } from '@/lib/test-runs'

const mocks = vi.hoisted(() => ({ definitions: vi.fn(), api: vi.fn(), error: vi.fn(), select: vi.fn() }))
vi.mock('@/hooks/useProjectUpdates', () => ({ useProjectUpdates: () => {} }))
vi.mock('@/hooks/useWorkspaceSection', () => ({ useWorkspaceSection: () => [null, mocks.select] }))
vi.mock('../specification/context', () => ({ useSpecification: () => ({ token: 'test', lang: 'en', project: 'demo', scopes: [], nodes: [{ id: 'n1', title: 'Billing' }], checks: [], editCheck: vi.fn(), refreshChecks: vi.fn(), onError: mocks.error }) }))
vi.mock('@/lib/test-runs', () => ({ listDefinitions: mocks.definitions }))
vi.mock('@/lib/api', () => ({ api: mocks.api }))
vi.mock('@/lib/initiatives', () => ({ listInitiatives: async () => ({ items: [] }) }))
vi.mock('@/lib/verification', () => ({ listEnvironments: async () => ({ items: [] }), archiveCheck: vi.fn(), createCheck: vi.fn() }))
vi.mock('@/components/verification/CheckDialog', () => ({ CheckDialog: () => null }))
vi.mock('./RunComposer', () => ({ RunComposer: () => null }))
vi.mock('./RunDetail', () => ({ RunDetail: () => null }))

function definition(id: string, title: string, state: string): TestDefinition {
  return { id, definition_revision: 'rev', specification_revision: null,
    definition: { id, title, node: 'n1', body: '', precondition: '', layer: 'ui', severity: 'advisory', verification: '', environments: [], cases: [] },
    execution: { state, environments: [] } }
}
beforeEach(() => {
  mocks.error.mockReset(); mocks.select.mockReset(); mocks.definitions.mockReset(); mocks.api.mockReset()
  mocks.definitions.mockResolvedValue([definition('c1', 'Invoice totals', 'failed'), definition('c2', 'Receipt preview', 'verified')])
  mocks.api.mockResolvedValue({ items: [], next_cursor: null, total: 0 })
})
function mount(search = '') { return render(<MemoryRouter initialEntries={[`/projects/demo/specification?view=tests${search}`]}><TestsView /></MemoryRouter>) }

it('narrows definitions by status and text while keeping a source-section link', async () => {
  mount()
  await screen.findByText('Invoice totals')
  expect(screen.getAllByRole('link', { name: 'Section: Billing' })[0]?.getAttribute('href')).toContain('section=n1')
  fireEvent.click(screen.getByRole('button', { name: 'Failed · 1' }))
  expect(screen.queryByText('Receipt preview')).toBeNull()
  fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'no match' } })
  expect(screen.getByText('No definitions match these filters.')).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }))
  expect(screen.getByText('Receipt preview')).toBeTruthy()
})

it('takes an empty Runs view to definitions', async () => {
  mount('&tests=runs')
  fireEvent.click(await screen.findByRole('button', { name: 'Go to definitions' }))
  expect(await screen.findByText('Invoice totals')).toBeTruthy()
})

it('shows one persistent permission error, no false empty state or repeated toast', async () => {
  mocks.definitions.mockRejectedValue(Object.assign(new Error('Access denied for demo'), { status: 403 }))
  mount('&tests=runs')
  expect((await screen.findByRole('alert')).textContent).toContain('Cannot access tests in demo')
  expect(screen.queryByText(/No runs on this page/)).toBeNull()
  expect(screen.queryByText('Invoice totals')).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: 'Refresh' }))
  await waitFor(() => expect(mocks.definitions).toHaveBeenCalledTimes(2))
  expect(screen.getAllByRole('alert')).toHaveLength(1)
  expect(mocks.error).not.toHaveBeenCalled()
})

it('preserves open test-case disclosures during a same-project refresh', async () => {
  mount()
  await screen.findByText('Invoice totals')
  const summary = screen.getAllByText('Test cases')[0]!
  fireEvent.click(summary)
  expect(summary.closest('details')?.open).toBe(true)
  fireEvent.click(screen.getByRole('button', { name: 'Refresh' }))
  await waitFor(() => expect(mocks.definitions).toHaveBeenCalledTimes(2))
  expect(screen.getAllByText('Test cases')[0]?.closest('details')?.open).toBe(true)
})
