import { beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter, useLocation } from 'react-router'
import type { ReactNode } from 'react'
import App from './App'
vi.mock('@/lib/initiatives', async original => ({
  ...(await original<typeof import('@/lib/initiatives')>()),
  whoami: vi.fn(async () => ({ actor: 'ada', scopes: ['read', 'write', 'human'] })),
  listProjects: vi.fn(async () => [{ id: 'demo', name: 'Demo' }, { id: 'other', name: 'Other' }]),
}))
vi.mock('@/components/AppShell', () => ({
  AppShell: ({ children, rail }: { children: ReactNode; rail: { project: string; onProject: (id: string) => void } }) => <><output data-testid="rail-project">{rail.project}</output><button onClick={() => rail.onProject('other')}>Pick other</button>{children}</>,
}))
vi.mock('./Queue', () => ({ Queue: ({ project }: { project: string }) => <output data-testid="queue">{project}</output> }))
function Probe() { return <output data-testid="search">{useLocation().search}</output> }
function mount(url: string) { render(<MemoryRouter initialEntries={[url]}><Probe /><App /></MemoryRouter>) }
beforeEach(() => { localStorage.clear(); localStorage.setItem('takomo.token', 'tk') })
describe('project in the bugs URL', () => {
  it('a link names the project regardless of the stored choice', async () => {
    localStorage.setItem('takomo.project', 'stored')
    mount('/bugs?project=demo&bug=demo-1')
    await waitFor(() => expect(screen.getByTestId('queue').textContent).toBe('demo'))
    expect(screen.getByTestId('rail-project').textContent).toBe('demo')
    expect(screen.getByTestId('search').textContent).toBe('?project=demo&bug=demo-1')
    expect(localStorage.getItem('takomo.project')).toBe('stored')
  })
  it('a bare /bugs carries the stored project into the URL so the page becomes shareable', async () => {
    localStorage.setItem('takomo.project', 'stored')
    mount('/bugs')
    await waitFor(() => expect(screen.getByTestId('search').textContent).toBe('?project=stored'))
    expect(screen.getByTestId('queue').textContent).toBe('stored')
  })
  it('picking a project resets the URL to that project and records the choice', async () => {
    mount('/bugs?project=demo&view=all&bug=demo-1')
    await screen.findByTestId('queue')
    fireEvent.click(screen.getByRole('button', {name:'Pick other'}))
    await waitFor(() => expect(screen.getByTestId('search').textContent).toBe('?project=other'))
    expect(screen.getByTestId('queue').textContent).toBe('other')
    expect(localStorage.getItem('takomo.project')).toBe('other')
  })
})
