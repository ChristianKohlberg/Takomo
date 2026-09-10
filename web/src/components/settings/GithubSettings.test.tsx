import { render, screen } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import { GithubSettings } from './GithubSettings'
import { githubStatus } from '@/lib/github'
vi.mock('@/lib/github', () => ({ githubStatus: vi.fn(), githubInstallations: vi.fn(), connectGithub: vi.fn(), disconnectGithub: vi.fn() }))
it('uses the verified account-specific installation management links', async () => {
 vi.mocked(githubStatus).mockResolvedValue({ configured: true, app_slug: 'takomo-test', connections: [
  { id: 1, account: 'person', management_url: 'https://github.com/settings/installations/1' },
  { id: 2, account: 'team', management_url: 'https://github.com/organizations/team/settings/installations/2' },
 ] })
 render(<GithubSettings token="test" locale="en" allowed />)
 const links = await screen.findAllByRole('link', { name: /Manage repositories and permissions/ })
 expect(links.map(l => l.getAttribute('href'))).toEqual(['https://github.com/settings/installations/1', 'https://github.com/organizations/team/settings/installations/2'])
})
