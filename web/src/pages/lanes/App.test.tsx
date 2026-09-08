import { beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter, useLocation } from 'react-router'
import App from './App'
vi.mock('@/lib/initiatives', () => ({ whoami: vi.fn(async () => ({ actor: 'human:test', scopes: ['read', 'write', 'human'] })), listProjects: vi.fn(async () => [{ id: 'demo' }, { id: 'linked-project' }]) }))
vi.mock('@/components/AppShell', () => ({ AppShell: ({ children, rail }: { children: React.ReactNode; rail: { onProject: (id: string) => void } }) => <div>{children}<button onClick={() => rail.onProject('demo')}>Choose demo</button></div> }))
function Location() { const location = useLocation(); return <div data-testid="location">{location.search}</div> }
vi.mock('./Workspace', () => ({ Workspace: ({ project }: { project: string }) => <div data-testid="workspace">{project}</div> }))
beforeEach(() => { localStorage.clear(); localStorage.setItem('takomo.token', 'secret'); localStorage.setItem('takomo.project', 'demo'); window.history.replaceState({}, '', '/lanes') })
describe('Lanes project links', () => {
 it('opens the linked project instead of a previously selected project', async () => { window.history.replaceState({}, '', '/lanes?project=linked-project'); render(<MemoryRouter><App /></MemoryRouter>); expect(await screen.findByTestId('workspace')).toHaveProperty('textContent', 'linked-project'); await waitFor(() => expect(localStorage.getItem('takomo.project')).toBe('linked-project')) })
 it('updates the project query when the picker changes', async () => { window.history.replaceState({}, '', '/lanes?project=linked-project'); render(<MemoryRouter initialEntries={['/lanes?project=linked-project']}><App /><Location /></MemoryRouter>); await screen.findByTestId('workspace'); fireEvent.click(screen.getByRole('button', { name: 'Choose demo' })); expect(screen.getByTestId('workspace')).toHaveProperty('textContent', 'demo'); expect(screen.getByTestId('location')).toHaveProperty('textContent', '?project=demo'); expect(localStorage.getItem('takomo.project')).toBe('demo') })
 it('ignores an invalid project parameter', async () => { window.history.replaceState({}, '', '/lanes?project=..%2Felsewhere'); render(<MemoryRouter><App /></MemoryRouter>); expect(await screen.findByTestId('workspace')).toHaveProperty('textContent', 'demo') })
})
