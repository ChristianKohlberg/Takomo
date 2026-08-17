// The two modes of one form, and the three refusals that must happen before a
// round trip.
//
// jsdom has no layout engine, so nothing here proves the dialog looks right. What
// it proves is the part that would be a silent data problem: the handle is offered
// when adding and only *shown* when editing (it is what every `person:<handle>`
// reference resolves through, so an editable box would invite orphaning them), a
// bad handle is refused locally rather than by a 422, an emptied email is sent as
// an explicit null rather than dropped, and the membership set is exactly what was
// picked.
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, screen, cleanup, fireEvent, waitFor } from '@testing-library/react'
import { PersonDialog } from './PersonDialog'
import type { User } from '@/lib/users'

afterEach(cleanup)

const LABELS = {
  addTitle: 'Add a person',
  addSubtitle: 'Somebody a decision can be addressed to.',
  editTitle: 'Edit person',
  editSubtitle: 'Their name, email and projects.',
  handle: 'Handle',
  handlePh: 'ada',
  handleHint: 'Lowercase, stable identity.',
  handleFixed: 'Fixed: references resolve through it.',
  name: 'Name',
  namePh: 'Ada Lovelace',
  nameHint: 'What a reader sees.',
  email: 'Email',
  emailPh: 'ada@example.com',
  emailHint: 'For a human reader.',
  projects: 'Can be handed work in',
  projectsHint: 'Membership says who may be handed work.',
  noProjectsPicked: 'No project picked — cannot be assigned anything.',
  save: 'Save',
  add: 'Add',
  cancel: 'Cancel',
  needHandle: 'A handle is required.',
  badHandle: 'A handle must be lowercase.',
}

const PROJECTS = [{ id: 'demo' }, { id: 'tp' }] as { id: string }[]

function mount(over: { person?: User | null; onSave?: () => Promise<unknown> } = {}) {
  const onSave = over.onSave ?? vi.fn().mockResolvedValue(undefined)
  render(
    <PersonDialog
      open
      onOpenChange={() => {}}
      person={over.person ?? null}
      // The page passes real projects; only `id` is read here.
      projects={PROJECTS as never}
      labels={LABELS}
      onSave={onSave}
    />,
  )
  return onSave
}

describe('PersonDialog, adding', () => {
  it('refuses an empty handle without calling the server', async () => {
    const onSave = mount()
    fireEvent.click(screen.getByRole('button', { name: 'Add' }))
    expect(await screen.findByText('A handle is required.')).toBeTruthy()
    expect(onSave).not.toHaveBeenCalled()
  })

  it('refuses a handle the server would reject, before the round trip', async () => {
    const onSave = mount()
    fireEvent.change(screen.getByLabelText('Handle'), { target: { value: 'Ada Lovelace' } })
    fireEvent.click(screen.getByRole('button', { name: 'Add' }))
    expect(await screen.findByText('A handle must be lowercase.')).toBeTruthy()
    expect(onSave).not.toHaveBeenCalled()
  })

  it('sends the handle, the name and exactly the projects picked', async () => {
    const onSave = mount()
    fireEvent.change(screen.getByLabelText('Handle'), { target: { value: 'ada' } })
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Ada Lovelace' } })
    fireEvent.click(screen.getByRole('button', { name: 'demo' }))
    fireEvent.click(screen.getByRole('button', { name: 'Add' }))
    await waitFor(() =>
      expect(onSave).toHaveBeenCalledWith({
        handle: 'ada',
        name: 'Ada Lovelace',
        // No address typed is an explicit "none", not an omitted field.
        email: null,
        projects: ['demo'],
      }),
    )
  })

  it('says what picking no project means, since it is a real state', () => {
    mount()
    expect(screen.getByText(/cannot be assigned anything/)).toBeTruthy()
  })
})

describe('PersonDialog, editing', () => {
  const ada: User = {
    id: 'usr-1',
    handle: 'ada',
    label: 'Ada Lovelace',
    name: 'Ada Lovelace',
    email: 'ada@example.com',
    projects: ['demo'],
  }

  it('shows the handle without offering to change it', () => {
    mount({ person: ada })
    // Present as text…
    expect(screen.getByText('ada')).toBeTruthy()
    expect(screen.getByText('Fixed: references resolve through it.')).toBeTruthy()
    // …and not as an editable box, which is the part that matters.
    expect(screen.queryByPlaceholderText('ada')).toBeNull()
  })

  it('opens on the person and carries their membership set', async () => {
    const onSave = mount({ person: ada })
    expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Ada Lovelace')
    expect(screen.getByRole('button', { name: 'demo' }).getAttribute('aria-pressed')).toBe('true')
    expect(screen.getByRole('button', { name: 'tp' }).getAttribute('aria-pressed')).toBe('false')

    // Add one project, drop the other: the save carries the resulting set, and the
    // page turns that into the two membership calls.
    fireEvent.click(screen.getByRole('button', { name: 'tp' }))
    fireEvent.click(screen.getByRole('button', { name: 'demo' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() =>
      expect(onSave).toHaveBeenCalledWith(
        expect.objectContaining({ handle: 'ada', projects: ['tp'] }),
      ),
    )
  })

  it('clears an email as an explicit null rather than dropping the field', async () => {
    const onSave = mount({ person: ada })
    fireEvent.change(screen.getByLabelText('Email'), { target: { value: '  ' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ email: null })))
  })

  it('surfaces a refusal from the server instead of closing on it', async () => {
    const onSave = vi.fn().mockRejectedValue(new Error('that project is archived'))
    mount({ person: ada, onSave })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    expect(await screen.findByText('that project is archived')).toBeTruthy()
  })
})
