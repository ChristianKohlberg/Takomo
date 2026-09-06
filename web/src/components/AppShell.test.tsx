import { beforeEach, describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { AppShell } from './AppShell'
import { AppHeader } from './AppHeader'
import { listQuestions } from '@/lib/questions'
import { loadToken } from '@/lib/session'
import { api } from '@/lib/api'
import { ProjectUpdatesContext } from '@/hooks/useProjectUpdates'

vi.mock('@/lib/questions', () => ({ listQuestions: vi.fn() }))
vi.mock('@/lib/session', () => ({ loadToken: vi.fn(() => '') }))
vi.mock('@/lib/api', () => ({ api: vi.fn() }))
beforeEach(() => { vi.mocked(loadToken).mockReturnValue(''); vi.mocked(listQuestions).mockReset(); vi.mocked(api).mockReset() })

const nav = { specification: 'Plan', board: 'Board', inbox: 'Inbox', documents: 'Documents', initiatives: 'Initiatives', mindmaps: 'Mindmaps', schedules: 'Schedules', verification: 'Verification', environments: 'Environments' }

type Updates = { project: string; subscribe: (callback: () => Promise<unknown>) => () => void }

function mount(collapsed: boolean, views = false, count: number | null = 4, updates: Updates | null = null) {
  const onLang = vi.fn()
  const onProject = vi.fn()
  const onNavigate = vi.fn()
  const onCollapsed = vi.fn()
  render(
    <ProjectUpdatesContext value={updates}>
    <AppShell lang="en" onLang={onLang} rail={{
      nav, current: 'board', collapsed, onCollapsed, onNavigate, onProject,
      onSignOut: vi.fn(), badges: { inbox: count ?? undefined },
      labels: { expand: 'Expand', collapse: 'Collapse', signOut: 'Sign out', account: 'Account', settings: 'Settings' },
      projects: [{ id: 'one', name: 'First project' }, { id: 'two', name: 'Second project' }],
      project: 'one', projectLabels: { project: 'Project', search: 'Search projects', noMatch: 'No matches' },
    }}>
      <AppHeader title="Board" views={views ? <span>Plan views</span> : undefined} />
    </AppShell>
    </ProjectUpdatesContext>,
  )
  return { onProject, onNavigate, onCollapsed, onLang }
}

describe('persistent rail navigation', () => {
  it.each([true, false])('keeps global controls in the rail with collapsed=%s', (collapsed) => {
    const { onProject, onNavigate, onCollapsed } = mount(collapsed)
    const header = within(screen.getByRole('complementary'))
    expect(screen.getAllByRole('link', { name: 'Inbox' })).toHaveLength(1)
    expect(header.getByText('4')).toBeTruthy()
    fireEvent.click(header.getByRole('link', { name: 'Inbox' }))
    expect(onNavigate).toHaveBeenCalledWith('/inbox')
    fireEvent.click(header.getByRole('button', { name: collapsed ? 'Expand' : 'Collapse' }))
    expect(onCollapsed).toHaveBeenCalledWith(!collapsed)
    fireEvent.click(header.getByRole('button', { name: 'Project: First project' }))
    expect(header.getByRole('combobox', { name: 'Search projects' })).toBe(document.activeElement)
    fireEvent.click(header.getByRole('option', { name: /Second project/ }))
    expect(onProject).toHaveBeenCalledWith('two')
  })

  it('keeps global controls visible alongside plan views', () => {
    mount(true, true)
    const header = within(screen.getByRole('complementary'))
    expect(header.getByRole('link', { name: 'Inbox' })).toBeTruthy()
    expect(header.getByRole('button', { name: 'Project: First project' })).toBeTruthy()
    expect(within(screen.getByRole('banner')).getByText('Plan views')).toBeTruthy()
    expect(within(screen.getByRole('banner')).queryByRole('link', { name: 'Inbox' })).toBeNull()
  })

  it('shows a green completion badge when no inbox tasks remain', () => {
    mount(false, false, 0)
    expect(screen.getByRole('img', { name: 'Inbox: 0' })).toBeTruthy()
    expect(screen.queryByText('0')).toBeNull()
  })

  it('changes language from the profile dropdown using the keyboard', () => {
    const { onLang } = mount(false)
    expect(screen.queryByRole('menuitem', { name: /Language/ })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Account' }))
    fireEvent.keyDown(screen.getByRole('menu'), { key: 'ArrowDown' })
    fireEvent.keyDown(screen.getByRole('menu'), { key: 'ArrowDown' })
    const language = screen.getByRole('menuitem', { name: /Language/ })
    expect(document.activeElement).toBe(language)
    fireEvent.click(language)
    expect(onLang).toHaveBeenCalledWith('de')
  })

  it('loads the project inbox count without showing completion before it arrives', async () => {
    vi.mocked(loadToken).mockReturnValue('token')
    let resolve!: (value: Awaited<ReturnType<typeof listQuestions>>) => void
    vi.mocked(listQuestions).mockReturnValue(new Promise((done) => { resolve = done }))
    mount(false, false, null)
    expect(screen.queryByRole('img', { name: 'Inbox: 0' })).toBeNull()
    expect(listQuestions).toHaveBeenCalledWith('token', { project: 'one', status: 'open' })
    await act(async () => { resolve([]) })
    expect(screen.getByRole('img', { name: 'Inbox: 0' })).toBeTruthy()
  })

  it('never opens a project socket of its own for the badge', async () => {
    vi.mocked(loadToken).mockReturnValue('token')
    vi.mocked(listQuestions).mockResolvedValue([])
    mount(false, false, null)
    await screen.findByRole('img', { name: 'Inbox: 0' })
    expect(api).not.toHaveBeenCalled()
  })

  it('refreshes the badge through a shared project subscription when the page has one', async () => {
    vi.mocked(loadToken).mockReturnValue('token')
    vi.mocked(listQuestions).mockResolvedValueOnce([]).mockResolvedValueOnce([{ id: 'q-1' }] as never)
    let callback: (() => Promise<unknown>) | null = null
    const subscribe = vi.fn((fn: () => Promise<unknown>) => { callback = fn; return () => {} })
    mount(false, false, null, { project: 'one', subscribe })
    await screen.findByRole('img', { name: 'Inbox: 0' })
    expect(subscribe).toHaveBeenCalledOnce()
    await act(async () => { await callback!() })
    expect(within(screen.getByRole('complementary')).getByText('1')).toBeTruthy()
    expect(api).not.toHaveBeenCalled()
  })

  it('removes the completion badge if a refresh fails', async () => {
    vi.mocked(loadToken).mockReturnValue('token')
    vi.mocked(listQuestions).mockResolvedValueOnce([]).mockRejectedValueOnce(new Error('offline'))
    mount(false, false, null)
    await screen.findByRole('img', { name: 'Inbox: 0' })
    fireEvent(window, new Event('focus'))
    await waitFor(() => expect(screen.queryByRole('img', { name: 'Inbox: 0' })).toBeNull())
  })

  it('preserves modified Inbox clicks for browser navigation', () => {
    const { onNavigate } = mount(true)
    fireEvent.click(screen.getByRole('link', { name: 'Inbox' }), { ctrlKey: true })
    expect(onNavigate).not.toHaveBeenCalled()
  })
})
