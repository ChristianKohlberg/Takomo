import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, within } from '@testing-library/react'
import { AppShell } from './AppShell'
import { AppHeader } from './AppHeader'

const nav = { specification: 'Plan', board: 'Board', inbox: 'Inbox', documents: 'Documents', initiatives: 'Initiatives', mindmaps: 'Mindmaps', schedules: 'Schedules', verification: 'Verification', environments: 'Environments' }

function mount(collapsed: boolean, views = false) {
  const onProject = vi.fn()
  const onNavigate = vi.fn()
  const onCollapsed = vi.fn()
  render(
    <AppShell rail={{
      nav, current: 'board', collapsed, onCollapsed, onNavigate, onProject,
      onSignOut: vi.fn(), badges: { inbox: 4 },
      labels: { expand: 'Expand', collapse: 'Collapse', signOut: 'Sign out', account: 'Account', settings: 'Settings' },
      projects: [{ id: 'one', name: 'First project' }, { id: 'two', name: 'Second project' }],
      project: 'one', projectLabels: { project: 'Project', search: 'Search projects', noMatch: 'No matches' },
    }}>
      <AppHeader title="Board" lang="en" onLang={vi.fn()} views={views ? <span>Plan views</span> : undefined} />
    </AppShell>,
  )
  return { onProject, onNavigate, onCollapsed }
}

describe('persistent header navigation', () => {
  it.each([true, false])('keeps global controls in the header with collapsed=%s', (collapsed) => {
    const { onProject, onNavigate, onCollapsed } = mount(collapsed)
    const header = within(screen.getByRole('banner'))
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
    const header = within(screen.getByRole('banner'))
    expect(header.getByRole('link', { name: 'Inbox' })).toBeTruthy()
    expect(header.getByRole('button', { name: 'Project: First project' })).toBeTruthy()
    expect(header.getByText('Plan views')).toBeTruthy()
  })

  it('preserves modified Inbox clicks for browser navigation', () => {
    const { onNavigate } = mount(true)
    fireEvent.click(screen.getByRole('link', { name: 'Inbox' }), { ctrlKey: true })
    expect(onNavigate).not.toHaveBeenCalled()
  })
})
