// What a layout test can and cannot see: jsdom has no layout engine, so nothing
// here proves the rail is 56px wide or that the phone overlay covers the page.
// What it CAN prove is the part that is behaviour rather than pixels — that a
// collapsed rail still exposes every destination by name, that the current
// surface is not a link to itself, and that a plain click is intercepted while
// a cmd-click is not.
import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import { NavRail, type NavRailProps } from './NavRail'

const LABELS = {
  expand: 'Expand',
  collapse: 'Collapse',
  signOut: 'Sign out',
  account: 'Account',
  settings: 'Settings',
}
const NAV = {
  board: 'Board', epics: 'Epics',
  inbox: 'Inbox',
  specification: 'Specification',
  initiatives: 'Initiatives',
  schedules: 'Schedules',
  environments: 'Environments',
}

function mount(props: Partial<NavRailProps> = {}) {
  const onCollapsed = vi.fn()
  const onSignOut = vi.fn()
  const onNavigate = vi.fn()
  render(
    <NavRail
      nav={NAV}
      current="board"
      labels={LABELS}
      collapsed={false}
      onCollapsed={onCollapsed}
      onSignOut={onSignOut}
      onNavigate={onNavigate}
      {...props}
    />,
  )
  return { onCollapsed, onSignOut, onNavigate }
}

const accountTrigger = () => screen.getByRole('button', { name: 'Account' })

describe('NavRail', () => {
  it('links to the other surfaces and not to the current one', () => {
    mount()
    expect(screen.getByRole('link', { name: 'Inbox' })).toHaveProperty(
      'pathname',
      '/inbox',
    )
    expect(screen.queryByRole('link', { name: 'Board' })).toBeNull()
    expect(screen.getByText('Board')).toBeTruthy()
  })

  it('keeps only primary destinations in the main navigation', () => {
    mount({ current: 'inbox' })
    for (const name of ['Specification', 'Document', 'Map', 'Verification and Evidence', 'Board', 'Lanes']) expect(screen.getByRole('link', { name })).toBeTruthy()
    for (const name of ['Bugs', 'Epics', 'Initiatives', 'Schedules', 'Environments', 'Agent queue']) expect(screen.queryByRole('link', { name })).toBeNull()
  })

  it('keeps every destination reachable by name when collapsed', () => {
    // The label is hidden, so `title`/`aria-label` is the only thing left — lose
    // it and a collapsed rail is a column of unlabelled glyphs to a screen reader.
    mount({ collapsed: true })
    for (const name of ['Specification', 'Document', 'Map', 'Verification and Evidence', 'Lanes', 'Inbox']) {
      expect(screen.getByRole('link', { name })).toBeTruthy()
    }
  })

  it('localizes the specification children', () => {
    mount({ lang: 'de', collapsed: true })
    expect(screen.getByRole('link', { name: 'Dokument' }).getAttribute('href')).toBe('/specification?view=document')
    expect(screen.getByRole('link', { name: 'Karte' }).getAttribute('href')).toBe('/specification?view=map')
  })

  it('renders the count when expanded', () => {
    mount({ badges: { inbox: 4 } })
    expect(screen.getByText('4')).toBeTruthy()
  })

  it('keeps the Inbox count when collapsed', () => {
    // The dot that replaces it is presentational, so the assertion is the
    // absence of the number rather than the presence of the dot.
    mount({ collapsed: true, badges: { inbox: 4 } })
    expect(screen.getByText('4')).toBeTruthy()
  })

  it('renders nothing for a zero badge', () => {
    mount({ badges: { inbox: 0 } })
    expect(screen.queryByText('0')).toBeNull()
  })

  it('toggles through the caller', () => {
    const { onCollapsed } = mount()
    screen.getByRole('button', { name: 'Collapse' }).click()
    expect(onCollapsed).toHaveBeenCalledWith(true)
  })

  it('opens the account menu with settings and sign-out', () => {
    mount()
    fireEvent.click(accountTrigger())
    const menu = screen.getByRole('menu')
    expect(menu).toBeTruthy()
    expect(screen.getByRole('menuitem', { name: 'Settings' })).toHaveProperty(
      'pathname',
      '/settings',
    )
    expect(screen.getByRole('menuitem', { name: 'Sign out' })).toBeTruthy()
  })

  it('navigates to settings through onNavigate on a plain click', () => {
    const { onNavigate } = mount()
    fireEvent.click(accountTrigger())
    fireEvent.click(screen.getByRole('menuitem', { name: 'Settings' }))
    expect(onNavigate).toHaveBeenCalledWith('/settings')
  })

  it('signs out from the account menu', () => {
    const { onSignOut } = mount()
    fireEvent.click(accountTrigger())
    fireEvent.click(screen.getByRole('menuitem', { name: 'Sign out' }))
    expect(onSignOut).toHaveBeenCalled()
  })

  it('places the account menu beside the trigger when the rail is collapsed', () => {
    mount({ collapsed: true })
    fireEvent.click(accountTrigger())
    expect(screen.getByRole('menu').className).toContain('left-full')
  })

  it('highlights the account trigger on /settings', () => {
    mount({ current: 'account' })
    expect(accountTrigger().className).toContain('bg-secondary')
  })

  it('shows the actor and derives a role from its scopes', () => {
    mount({ actor: 'human:ada', scopes: ['read', 'write', 'human'] })
    expect(screen.getByText('human:ada')).toBeTruthy()
    expect(screen.getByText('human')).toBeTruthy()
  })

  it('falls back to the account label when whoami has not answered', () => {
    mount()
    expect(screen.getAllByText('Account').length).toBeGreaterThan(0)
  })
})

// Project context must survive both normal navigation and copy-link/new-tab.
it('carries the current project into Lanes destinations', () => {
  const { onNavigate } = mount({ project: 'demo' })
  const lanes = screen.getByRole('link', { name: 'Lanes' })
  expect(lanes.getAttribute('href')).toBe('/lanes?project=demo')
  fireEvent.click(lanes)
  expect(onNavigate).toHaveBeenCalledWith('/lanes?project=demo')
})

it('preserves project and selected section across all child views and modified clicks', () => {
  const { onNavigate } = mount({ current: 'specification', project: 'project / one', specificationView: 'map', specificationSection: 'node/one' })
  for (const [name, view] of [['Document', 'document'], ['Map', 'map'], ['Verification and Evidence', 'tests']]) {
    const link = screen.getByRole('link', { name })
    expect(link.getAttribute('href')).toBe(`/projects/project%20%2F%20one/specification?view=${view}&section=node%2Fone`)
    fireEvent.click(link)
    expect(onNavigate).toHaveBeenLastCalledWith(link.getAttribute('href'))
  }
  expect(screen.getByRole('link', { name: 'Map' }).getAttribute('aria-current')).toBe('page')
  onNavigate.mockClear()
  fireEvent.click(screen.getByRole('link', { name: 'Verification and Evidence' }), { ctrlKey: true })
  expect(onNavigate).not.toHaveBeenCalled()
})

it('places one Inbox link above Profile and keeps scoped Settings in its menu', () => {
  mount({ project: 'demo', navigationInHeader: true })
  const inbox = screen.getByRole('link', { name: 'Inbox' })
  expect(screen.queryByRole('link', { name: 'Settings' })).toBeNull()
  fireEvent.click(accountTrigger())
  const settings = screen.getByRole('menuitem', { name: 'Settings' })
  expect(screen.getAllByRole('link', { name: 'Inbox' })).toHaveLength(1)
  expect(settings.getAttribute('href')).toBe('/settings?scope=demo')
  expect(inbox.compareDocumentPosition(accountTrigger()) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  expect(inbox.closest('nav')).toBeNull()
})
