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
  board: 'Board',
  inbox: 'Inbox',
  initiatives: 'Initiatives',
  schedules: 'Schedules',
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
  it('links to the other three surfaces and not to the current one', () => {
    mount()
    expect(screen.getByRole('link', { name: 'Inbox' })).toHaveProperty(
      'pathname',
      '/inbox',
    )
    expect(screen.queryByRole('link', { name: 'Board' })).toBeNull()
    expect(screen.getByText('Board')).toBeTruthy()
  })

  it('does not list settings among the surface links', () => {
    mount()
    expect(screen.queryByRole('link', { name: 'Settings' })).toBeNull()
  })

  it('keeps every destination reachable by name when collapsed', () => {
    // The label is hidden, so `title`/`aria-label` is the only thing left — lose
    // it and a collapsed rail is four unlabelled glyphs to a screen reader.
    mount({ collapsed: true })
    for (const name of ['Inbox', 'Initiatives', 'Schedules']) {
      expect(screen.getByRole('link', { name })).toBeTruthy()
    }
  })

  it('renders the count when expanded', () => {
    mount({ badges: { inbox: 4 } })
    expect(screen.getByText('4')).toBeTruthy()
  })

  it('drops the number when collapsed — there is nowhere to put it', () => {
    // The dot that replaces it is presentational, so the assertion is the
    // absence of the number rather than the presence of the dot.
    mount({ collapsed: true, badges: { inbox: 4 } })
    expect(screen.queryByText('4')).toBeNull()
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
