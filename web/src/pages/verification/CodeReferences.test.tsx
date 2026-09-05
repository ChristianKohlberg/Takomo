import { render, screen, within } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { expect, it, vi } from 'vitest'
import { TestsView } from './App'

vi.mock('@/hooks/useProjectUpdates', () => ({ useProjectUpdates: () => {} }))
vi.mock('@/hooks/useWorkspaceSection', () => ({ useWorkspaceSection: () => [null, vi.fn()] }))
vi.mock('../specification/context', () => ({ useSpecification: () => ({
  token: 'token', lang: 'en', project: 'p', scopes: [], nodes: [],
  checks: [{ id: 'linked', metadata: { specification: { bindings: [{
    file: 'tests/navigation.ts', selector: 'keeps my place', proves: 'The section stays selected.', limits: 'No real browser.',
  }] } } }],
  editCheck: vi.fn(), refreshChecks: vi.fn(), onError: vi.fn(),
}) }))
vi.mock('@/lib/api', () => ({ api: vi.fn(async () => ({ items: [], next_cursor: null, total: 0 })) }))
vi.mock('@/lib/initiatives', () => ({ listInitiatives: vi.fn(async () => ({ items: [] })) }))
vi.mock('@/lib/verification', async importOriginal => ({
  ...await importOriginal<typeof import('@/lib/verification')>(),
  listEnvironments: vi.fn(async () => ({ items: [] })),
}))
vi.mock('@/lib/test-runs', async importOriginal => ({
  ...await importOriginal<typeof import('@/lib/test-runs')>(),
  listDefinitions: vi.fn(async () => ['unlinked', 'linked'].map(id => ({
    id, definition: { title: id, layer: 'ui', severity: 'advisory', cases: [{ id: `${id}-case`, key: 'navigation', label: 'Keep my place', assignment: { steps: ['Open the map.'], expected: 'My section stays selected.' } }] },
    execution: { state: 'not_executed', environments: [] },
  }))),
}))

it('matches references to their check ID without changing the execution state', async () => {
  render(<MemoryRouter><TestsView /></MemoryRouter>)
  const linked = (await screen.findByRole('heading', { name: 'linked' })).closest('article')!
  const unlinked = screen.getByRole('heading', { name: 'unlinked' }).closest('article')!
  expect(within(linked).getByText('Code references (1)')).toBeTruthy()
  expect(within(linked).getByText('keeps my place')).toBeTruthy()
  expect(within(linked).getByText('Not run')).toBeTruthy()
  expect(within(linked).getByText('Open the map.')).toBeTruthy()
  expect(within(linked).getByText('My section stays selected.')).toBeTruthy()
  expect(within(unlinked).queryByText(/Code references/)).toBeNull()
  expect(within(unlinked).getByText('Not run')).toBeTruthy()
})
