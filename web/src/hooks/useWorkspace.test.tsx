import { useState } from 'react'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { MemoryRouter, useLocation, useNavigate } from 'react-router'
import { afterEach, expect, it } from 'vitest'
import { useWorkspaceProject, useWorkspaceNavigate } from './useWorkspace'
function Probe() {
  const [project, select] = useWorkspaceProject()
  const location = useLocation()
  const navigate = useWorkspaceNavigate()
  const history = useNavigate()
  return (
    <>
      <output>{location.pathname + location.search + location.hash}</output>
      <Panel key={project} />
      <button onClick={() => select('b')}>Project B</button>
      <button onClick={() => navigate('/documents#n=section')}>Document</button>
      <button onClick={() => history(-1)}>Back</button>
    </>
  )
}
function Panel() {
  const [value, setValue] = useState('')
  return <input aria-label="draft" value={value} onChange={(e) => setValue(e.target.value)} />
}
afterEach(() => {
  cleanup()
  localStorage.clear()
})
it('clears project-specific state and hashes on scope changes and restores scope on Back', () => {
  render(
    <MemoryRouter
      initialEntries={['/projects/a/specification?view=tests&check=check-a&section=node-a']}
    >
      <Probe />
    </MemoryRouter>,
  )
  fireEvent.change(screen.getByLabelText('draft'), { target: { value: 'old selection' } })
  fireEvent.click(screen.getByText('Project B'))
  expect(screen.getByRole('status').textContent).toBe('/projects/b/specification?view=tests')
  expect((screen.getByLabelText('draft') as HTMLInputElement).value).toBe('')
  fireEvent.click(screen.getByText('Back'))
  expect(screen.getByRole('status').textContent).toBe(
    '/projects/a/specification?view=tests&check=check-a&section=node-a',
  )
  fireEvent.click(screen.getByText('Document'))
  expect(screen.getByRole('status').textContent).toBe(
    '/projects/a/specification?view=document&section=section',
  )
})
