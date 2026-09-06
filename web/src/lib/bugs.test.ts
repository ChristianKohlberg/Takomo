import { beforeEach, expect, it, vi } from 'vitest'
import { api } from './api'
import { listBugs } from './bugs'
vi.mock('./api', () => ({api: vi.fn()}))
beforeEach(() => vi.resetAllMocks())
it('filters and paginates on the server so matches beyond the first page remain visible', async () => {
  await listBugs('tk', 'demo', 'in_progress', 'high', 'receipt failure', 50)
  const query = new URL(vi.mocked(api).mock.calls[0]![1], 'http://test').searchParams
  expect(Object.fromEntries(query)).toEqual({project:'demo', view:'in_progress', severity:'high', search:'receipt failure', limit:'50', offset:'50'})
  expect(query.has('triage')).toBe(false)
})
it('requests open bugs by default and sends All as an explicit view', async () => {
  await listBugs('tk', 'demo', 'open', '', '', 0)
  await listBugs('tk', 'demo', 'all', '', '', 0)
  expect(vi.mocked(api).mock.calls[0]![1]).toContain('view=open')
  expect(vi.mocked(api).mock.calls[1]![1]).toContain('view=all')
})
