import { fireEvent, render, screen, cleanup } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, it, vi } from 'vitest'
import { ViewSwitcher } from './ViewSwitcher'
afterEach(() => { cleanup(); localStorage.clear() })
it('carries the current project and section to every view instead of using another tab’s project', () => {
  localStorage.setItem('takomo.project', 'other-tab')
  const navigate = vi.fn()
  render(<MemoryRouter initialEntries={['/mindmaps?project=p#m=mm-one&n=mn-one']}><ViewSwitcher current="map" labels={{map:'Map',document:'Document',tests:'Tests'}} onNavigate={navigate} /></MemoryRouter>)
  expect(screen.getByRole('link', {name:'Document'}).getAttribute('href')).toBe('/documents?project=p#n=mn-one')
  expect(screen.getByRole('link', {name:'Tests'}).getAttribute('href')).toBe('/verification?project=p#n=mn-one')
  expect(screen.getByRole('link', {name:'Map'}).getAttribute('href')).toBe('/mindmaps?project=p#m=mm-one&n=mn-one')
  fireEvent.click(screen.getByRole('link', {name:'Document'}))
  expect(navigate).toHaveBeenCalledWith('/documents?project=p#n=mn-one')
})
