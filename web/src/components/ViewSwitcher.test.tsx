import { fireEvent, render, screen, cleanup } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, it, vi } from 'vitest'
import { ViewSwitcher } from './ViewSwitcher'
afterEach(() => { cleanup(); sessionStorage.clear(); localStorage.clear() })
it('restores bookmarked selections and names every compact navigation link', () => {
  localStorage.setItem('takomo.project', 'p')
  sessionStorage.setItem('takomo.workspace:p:/documents', '#n=mn-one')
  const navigate = vi.fn()
  render(<MemoryRouter initialEntries={['/mindmaps#m=mm-one']}><ViewSwitcher current="map" labels={{map:'Map',document:'Document',tests:'Tests'}} onNavigate={navigate} /></MemoryRouter>)
  expect(screen.getByRole('link', {name:'Document'}).getAttribute('href')).toBe('/documents#n=mn-one')
  expect(screen.getByRole('link', {name:'Map'}).getAttribute('href')).toBe('/mindmaps#m=mm-one')
  fireEvent.click(screen.getByRole('link', {name:'Document'}))
  expect(navigate).toHaveBeenCalledWith('/documents#n=mn-one')
})
