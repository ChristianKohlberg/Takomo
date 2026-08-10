import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import { ProjectPicker } from './ProjectPicker'

const PROJECTS = [
  { id: 'demo', name: 'Demo — agent fleet' },
  { id: 'billing', name: 'Billing platform' },
  { id: 'infra', name: null },
]

const LABELS = {
  project: 'project',
  search: 'Search projects',
  noMatch: 'No project matches “{q}”',
  all: 'All projects',
}

function mount(props: Partial<Parameters<typeof ProjectPicker>[0]> = {}) {
  const onChange = vi.fn()
  render(
    <ProjectPicker projects={PROJECTS} value="" onChange={onChange} labels={LABELS} {...props} />,
  )
  return { onChange }
}

const trigger = () => screen.getByRole('button', { name: /^project:/ })
const search = () => screen.getByRole('combobox')

describe('ProjectPicker', () => {
  it('names the current scope on the trigger, so the rail says what you are looking at', () => {
    mount({ value: 'demo' })
    expect(trigger().textContent).toContain('Demo — agent fleet')
  })

  it('falls back to the id when a project has no name', () => {
    mount({ value: 'infra' })
    expect(trigger().textContent).toContain('infra')
  })

  it('opens on click and filters as you type', () => {
    mount()
    fireEvent.click(trigger())
    expect(screen.getByRole('option', { name: /Billing platform/ })).toBeTruthy()

    fireEvent.change(search(), { target: { value: 'bill' } })
    expect(screen.queryByRole('option', { name: /Demo — agent fleet/ })).toBeNull()
    expect(screen.getByRole('option', { name: /Billing platform/ })).toBeTruthy()
  })

  it('searches the id too, not only the name', () => {
    mount()
    fireEvent.click(trigger())
    fireEvent.change(search(), { target: { value: 'infra' } })
    expect(screen.getByRole('option', { name: /infra/ })).toBeTruthy()
  })

  it('says so when nothing matches, quoting what was typed', () => {
    mount()
    fireEvent.click(trigger())
    fireEvent.change(search(), { target: { value: 'zzz' } })
    expect(screen.getByText('No project matches “zzz”')).toBeTruthy()
  })

  it('picks with the keyboard — the all-projects row included', () => {
    // The "all" row is index 0, so Enter on a fresh popover selects it. If it
    // were click-only it would be unreachable to a keyboard reader.
    const { onChange } = mount({ value: 'demo' })
    fireEvent.click(trigger())
    fireEvent.keyDown(search(), { key: 'Enter' })
    expect(onChange).toHaveBeenCalledWith('')
  })

  it('arrows down to a project and takes it with Enter', () => {
    const { onChange } = mount()
    fireEvent.click(trigger())
    fireEvent.keyDown(search(), { key: 'ArrowDown' }) // off "all", onto demo
    fireEvent.keyDown(search(), { key: 'Enter' })
    expect(onChange).toHaveBeenCalledWith('demo')
  })

  it('closes on Escape without changing the scope', () => {
    const { onChange } = mount()
    fireEvent.click(trigger())
    fireEvent.keyDown(search(), { key: 'Escape' })
    expect(screen.queryByRole('listbox')).toBeNull()
    expect(onChange).not.toHaveBeenCalled()
  })

  it('offers no all-projects row when the label is withheld — /board relies on this', () => {
    const { all: _all, ...noAll } = LABELS
    mount({ labels: noAll })
    fireEvent.click(trigger())
    expect(screen.queryByRole('option', { name: 'All projects' })).toBeNull()
    expect(screen.getAllByRole('option')).toHaveLength(3)
  })

  it('collapses to a single initial', () => {
    mount({ value: 'demo', collapsed: true })
    expect(trigger().textContent).toBe('D')
  })
})
