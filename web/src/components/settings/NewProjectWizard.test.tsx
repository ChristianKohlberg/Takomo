import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { NewProjectWizard } from './NewProjectWizard'
import { createProject } from '@/lib/admin'
import { githubWrite, setRepository, startExtraction } from '@/lib/github'
vi.mock('@/lib/admin', () => ({ createProject: vi.fn() }))
vi.mock('@/lib/github', () => ({ githubWrite: vi.fn(), setRepository: vi.fn(), startExtraction: vi.fn() }))
vi.mock('./RepositoryFields', () => ({ RepositoryFields: ({ onChange }: { onChange: (v: unknown) => void }) => <button onClick={() => onChange({ installation: 1, repository: 2, full_name: 'owner/repo', scope: { include: ['src/checkout'], exclude: [] } })}>Select fixture repository</button> }))
beforeEach(() => { vi.clearAllMocks(); vi.mocked(createProject).mockResolvedValue({}); vi.mocked(setRepository).mockResolvedValue({}); vi.mocked(githubWrite).mockResolvedValue({ mindmap: { id: 'mm-test' } }); vi.mocked(startExtraction).mockResolvedValue({ id: 'ci-test' }) })
function mount() { const onCreated = vi.fn(); render(<NewProjectWizard open token="test-token" locale="en" initialName="checkout" canConnect onOpenChange={vi.fn()} onCreated={onCreated} />); return onCreated }
function next() { fireEvent.click(screen.getByRole('button', { name: 'Continue' })) }
it('creates an empty project without connecting GitHub or starting inference', async () => {
 const done = mount(); next(); next();
 expect(screen.getByText(/Review, edit and confirm them in the existing document or mindmap/)).toBeTruthy()
 fireEvent.click(screen.getByRole('button', { name: 'Create project' }))
 await waitFor(() => expect(done).toHaveBeenCalledWith('checkout'))
 expect(setRepository).not.toHaveBeenCalled(); expect(startExtraction).not.toHaveBeenCalled()
})
it('connecting a repository does not authorize extraction by itself', async () => {
 const done = mount(); next(); fireEvent.click(screen.getByLabelText('Connect a GitHub repository')); fireEvent.click(screen.getByText('Select fixture repository')); next();
 expect((screen.getByLabelText('Start extraction after creating the project') as HTMLInputElement).checked).toBe(false)
 fireEvent.click(screen.getByRole('button', { name: 'Create project' }))
 await waitFor(() => expect(done).toHaveBeenCalled())
 expect(setRepository).toHaveBeenCalled(); expect(startExtraction).not.toHaveBeenCalled()
})
it('shows a missing source path and starts no extraction when verification fails', async () => {
 vi.mocked(setRepository).mockRejectedValueOnce(new Error("Source path 'src/checkout' was not found on the selected repository's default branch."))
 mount(); next(); fireEvent.click(screen.getByLabelText('Connect a GitHub repository')); fireEvent.click(screen.getByText('Select fixture repository')); next();
 fireEvent.click(screen.getByLabelText('Start extraction after creating the project'))
 fireEvent.click(screen.getByRole('button', { name: 'Create and start extraction' }))
 expect((await screen.findByRole('alert')).textContent).toContain("Source path 'src/checkout' was not found")
 expect(startExtraction).not.toHaveBeenCalled()
 expect(githubWrite).not.toHaveBeenCalled()
})
it('starts one extraction only after explicit opt-in and reuses the project after a connection failure', async () => {
 vi.mocked(setRepository).mockRejectedValueOnce(new Error('GitHub access changed'))
 const done = mount(); next(); fireEvent.click(screen.getByLabelText('Connect a GitHub repository')); fireEvent.click(screen.getByText('Select fixture repository')); next();
 fireEvent.click(screen.getByLabelText('Start extraction after creating the project'))
 fireEvent.click(screen.getByRole('button', { name: 'Create and start extraction' }))
 await screen.findByRole('alert'); expect(startExtraction).not.toHaveBeenCalled()
 fireEvent.click(screen.getByRole('button', { name: 'Create and start extraction' }))
 await waitFor(() => expect(done).toHaveBeenCalled())
 expect(createProject).toHaveBeenCalledTimes(1); expect(startExtraction).toHaveBeenCalledTimes(1)
 expect(startExtraction).toHaveBeenCalledWith('test-token', 'checkout', 'mm-test', expect.any(String))
})
