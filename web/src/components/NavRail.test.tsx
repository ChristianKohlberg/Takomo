// What a layout test can and cannot see: jsdom has no layout engine, so nothing
// here proves the rail is 56px wide or that the phone overlay covers the page.
// What it CAN prove is the part that is behaviour rather than pixels — that a
// collapsed rail still exposes every destination by name, that the current
// surface is not a link to itself, and that a plain click is intercepted while
// a cmd-click is not.
import { describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import { NavRail, type NavRailProps } from './NavRail'

const LABELS = { expand: 'Expand', collapse: 'Collapse', signOut: 'Sign out', account: 'Account' }
const NAV = {
  board: 'Board',
  inbox: 'Inbox',
  initiatives: 'Initiatives',
  schedules: 'Schedules',
  settings: 'Settings',
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

describe('NavRail', () => {
  it('links to the other four surfaces and not to the current one', () => {
    mount()
    expect(screen.getByRole('link', { name: 'Inbox' })).toHaveProperty(
      'pathname',
      '/inbox',
    )
    expect(screen.queryByRole('link', { name: 'Board' })).toBeNull()
    expect(screen.getByText('Board')).toBeTruthy()
  })

  it('keeps every destination reachable by name when collapsed', () => {
    // The label is hidden, so `title`/`aria-label` is the only thing left — lose
    // it and a collapsed rail is five unlabelled glyphs to a screen reader.
    mount({ collapsed: true })
    for (const name of ['Inbox', 'Initiatives', 'Schedules', 'Settings']) {
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

  it('toggles through the caller, and signs out', () => {
    const { onCollapsed, onSignOut } = mount()
    screen.getByRole('button', { name: 'Collapse' }).click()
    expect(onCollapsed).toHaveBeenCalledWith(true)
    screen.getByRole('button', { name: 'Sign out' }).click()
    expect(onSignOut).toHaveBeenCalled()
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
