import { cleanup, render, screen } from '@testing-library/react'
import { MemoryRouter, Route, Routes, useLocation } from 'react-router'
import { afterEach, expect, it, vi } from 'vitest'
import { LegacySpecificationRedirect } from './LegacyRedirect'
import { getMindmap } from '@/lib/mindmaps'

vi.mock('@/lib/mindmaps', () => ({ getMindmap: vi.fn() }))
afterEach(() => { cleanup(); localStorage.clear(); vi.resetAllMocks() })
function Target() { const location = useLocation(); return <output>{location.pathname + location.search}</output> }
function open(path: string) {
  render(<MemoryRouter initialEntries={[path]}><Routes>
    <Route path="/mindmaps" element={<LegacySpecificationRedirect />} />
    <Route path="/verification" element={<LegacySpecificationRedirect />} />
    <Route path="/projects/:project/specification" element={<Target />} />
  </Routes></MemoryRouter>)
}
it('resolves the map’s actual project before redirecting a bookmarked section', async () => {
  localStorage.setItem('takomo.token', 'token')
  localStorage.setItem('takomo.project', 'another-tab')
  vi.mocked(getMindmap).mockResolvedValue({ mindmap: { project: 'actual' } } as Awaited<ReturnType<typeof getMindmap>>)
  open('/mindmaps?project=wrong#m=mm-shared&n=mn-selected')
  expect(await screen.findByText('/projects/actual/specification?view=map&section=mn-selected')).toBeTruthy()
  expect(getMindmap).toHaveBeenCalledWith('token', 'mm-shared')
})
it('preserves project, section and check on an old test link', async () => {
  open('/verification?project=actual#n=mn-selected&c=check-one')
  expect(await screen.findByText('/projects/actual/specification?view=tests&section=mn-selected&check=check-one')).toBeTruthy()
})
it('keeps a signed-out map bookmark in place until authentication', () => {
  open('/mindmaps#m=mm-shared&n=mn-selected')
  expect(screen.getByText('Sign in to open this specification.')).toBeTruthy()
  expect(getMindmap).not.toHaveBeenCalled()
})
