import { useState } from 'react'
import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { RepositoryFields } from './RepositoryFields'
import { githubRepositories, githubStatus, type RepositorySelection } from '@/lib/github'

vi.mock('@/lib/github', () => ({ githubRepositories: vi.fn(), githubStatus: vi.fn() }))
beforeEach(() => {
  vi.clearAllMocks()
  vi.mocked(githubStatus).mockResolvedValue({ configured: true, app_slug: 'test', connections: [{ id: 1, account: 'sample' }] })
  vi.mocked(githubRepositories).mockResolvedValue({ items: [{ id: 2, full_name: 'sample/Takomo', private: false }], total: 1 })
})
it('selects only the sample file while retaining the repository and allowing custom scope', async () => {
  const changed = vi.fn()
  function Harness() {
    const [value, setValue] = useState<RepositorySelection | null>({ installation: 1, repository: 2, full_name: 'sample/Takomo', scope: { include: ['src'], exclude: [] } })
    return <RepositoryFields token="test" locale="en" value={value} onChange={next => { changed(next); setValue(next) }} />
  }
  render(<Harness />)
  await screen.findByRole('option', { name: 'sample/Takomo' })
  fireEvent.click(screen.getByRole('button', { name: 'Use sample file' }))
  expect(changed).toHaveBeenLastCalledWith({ installation: 1, repository: 2, full_name: 'sample/Takomo', scope: { include: ['examples/extraction-fixture/checkout.mjs'], exclude: [] } })
  expect((screen.getByLabelText('File or folder to extract') as HTMLInputElement).value).toBe('examples/extraction-fixture/checkout.mjs')
  expect(screen.getByText(/This does not start extraction/)).toBeTruthy()
  fireEvent.change(screen.getByLabelText('File or folder to extract'), { target: { value: 'src/checkout' } })
  expect(changed.mock.lastCall?.[0].scope.include).toEqual(['src/checkout'])
})
it('requires a repository selection before offering the sample scope', async () => {
  render(<RepositoryFields token="test" locale="en" value={null} onChange={vi.fn()} />)
  await screen.findByRole('option', { name: 'sample' })
  expect(screen.queryByRole('button', { name: 'Use sample file' })).toBeNull()
})
