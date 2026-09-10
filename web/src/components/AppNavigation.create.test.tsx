import { fireEvent, render, screen } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import { NavRail, type NavRailProps } from './NavRail'
it('the actual header-integrated rail offers project creation to admins', () => {
 const onNavigate = vi.fn()
 const props: NavRailProps = { nav: { epics: '', board: 'Board', inbox: 'Inbox', specification: 'Specification', initiatives: '', schedules: '', environments: '' }, current: 'board', labels: { expand: 'Expand', collapse: 'Collapse', signOut: 'Sign out', account: 'Account', settings: 'Settings' }, collapsed: false, onCollapsed: vi.fn(), onSignOut: vi.fn(), onNavigate, scopes: ['admin'], projects: [{ id: 'demo' }], project: 'demo', projectLabels: { project: 'Project', search: 'Search projects', noMatch: 'No matches' } }
 render(<NavRail {...props} navigationInHeader />)
 fireEvent.click(screen.getByRole('button', { name: 'Project: demo' }))
 fireEvent.change(screen.getByRole('combobox'), { target: { value: 'New billing' } })
 fireEvent.keyDown(screen.getByRole('combobox'), { key: 'Enter' })
 expect(onNavigate).toHaveBeenCalledWith('/settings?section=projects&create=1&name=New%20billing')
})
