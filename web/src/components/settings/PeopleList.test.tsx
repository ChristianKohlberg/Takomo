// What an operator has to be able to read off a row of the directory.
//
// jsdom has no layout engine, so nothing here proves the row LOOKS right. What
// it can prove are the three facts an admin acts on, each of which has a wrong
// answer that would be invisible: the handle is shown (it is what every
// `person:<handle>` reference and every CLI command takes — the display name is
// not addressable), somebody who is a member of nothing says so (they cannot be
// handed work anywhere), and the action offered flips with the person's state
// rather than always reading "Disable".
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, screen, cleanup, fireEvent } from '@testing-library/react'
import { PeopleList } from './PeopleList'
import type { User } from '@/lib/users'

afterEach(cleanup)

const LABELS = {
  person: 'Person',
  projects: 'Can be handed work in',
  noProjects: 'no project yet',
  status: 'Status',
  active: 'Active',
  disabled: 'Disabled',
  disable: 'Disable',
  enable: 'Enable',
  disableHint: 'Stops new work being addressed to them.',
}

function person(over: Partial<User> = {}): User {
  return {
    id: 'usr-1',
    handle: 'ada',
    label: 'Ada Lovelace',
    projects: ['demo'],
    ...over,
  }
}

describe('PeopleList', () => {
  it('shows the handle as well as the display name', () => {
    render(<PeopleList people={[person()]} labels={LABELS} />)
    expect(screen.getByText('Ada Lovelace')).toBeTruthy()
    // The addressable identity, not just the pretty one.
    expect(screen.getByText(/^ada$/)).toBeTruthy()
  })

  it('says so when somebody is a member of nothing', () => {
    render(<PeopleList people={[person({ projects: [] })]} labels={LABELS} />)
    expect(screen.getByText('no project yet')).toBeTruthy()
  })

  it('offers Enable for a disabled person, and Disable for an active one', () => {
    const onSetDisabled = vi.fn()
    render(
      <PeopleList
        people={[person(), person({ id: 'usr-2', handle: 'sam', label: 'Sam', disabled: true })]}
        labels={LABELS}
        onSetDisabled={onSetDisabled}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Disable' }))
    expect(onSetDisabled).toHaveBeenCalledWith(expect.objectContaining({ handle: 'ada' }), true)
    fireEvent.click(screen.getByRole('button', { name: 'Enable' }))
    expect(onSetDisabled).toHaveBeenCalledWith(expect.objectContaining({ handle: 'sam' }), false)
  })

  it('hides the actions from a reader who cannot administer the directory', () => {
    render(<PeopleList people={[person()]} labels={LABELS} />)
    expect(screen.queryByRole('button')).toBeNull()
  })
})
