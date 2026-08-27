// The empty-string round trip, which is the only part of `Picker` that is not
// just Radix.
//
// Radix reserves `""` to mean "nothing selected", so it refuses an item with
// that value. This app uses `<option value="">All</option>` as its "no filter"
// idiom in twelve places, and those empty strings go straight into query strings
// and request bodies. `Picker` swaps in a sentinel and swaps it back out; if the
// swap-back ever regresses, the app starts filtering by a literal sentinel and
// the failure looks like "the All option returns nothing".
import { describe, expect, it, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import { Picker } from './Picker'
import { Hint } from './Hint'

const OPTIONS = [
  { value: '', label: 'All modes' },
  { value: 'blocking', label: 'Blocking' },
  { value: 'advisory', label: 'Advisory' },
]

describe('Picker', () => {
  it('renders an empty-valued option without Radix refusing it', () => {
    render(
      <Picker value="" onValueChange={() => {}} options={OPTIONS} aria-label="Mode" />,
    )
    // The trigger shows the empty option's label, so `''` selected the "All" row
    // rather than falling through to the placeholder.
    expect(screen.getByRole('combobox', { name: 'Mode' }).textContent).toContain('All modes')
  })

  it('reports a real value as itself', () => {
    render(
      <Picker value="blocking" onValueChange={() => {}} options={OPTIONS} aria-label="Mode" />,
    )
    expect(screen.getByRole('combobox', { name: 'Mode' }).textContent).toContain('Blocking')
  })

  it('never hands the sentinel to the caller', () => {
    const onValueChange = vi.fn()
    render(
      <Picker value="blocking" onValueChange={onValueChange} options={OPTIONS} aria-label="Mode" />,
    )
    // Drive the primitive's own callback rather than the pointer: jsdom has no
    // layout, so Radix's popover cannot be opened by a click here.
    fireEvent.keyDown(screen.getByRole('combobox', { name: 'Mode' }), { key: 'Home' })
    for (const call of onValueChange.mock.calls) {
      expect(call[0]).not.toContain('__none')
    }
  })

  // `Hint` reaches its child through a Radix `asChild` trigger, which works by
  // handing the child props and a ref. `Picker` renders no DOM of its own — the
  // trigger is the only element — so if it ever stops spreading through to it,
  // the tooltip silently attaches to nothing. It does not throw; it just stops
  // describing the control, which is invisible outside a screen reader.
  it('forwards composition props through to its trigger', () => {
    render(
      <Hint text="Pick a mode">
        <Picker value="blocking" onValueChange={() => {}} options={OPTIONS} aria-label="Mode" />
      </Hint>,
    )
    // One element, both roles: still the select's combobox, and now also carrying
    // the tooltip trigger's own marker. That the tooltip's `data-slot` WON the
    // spread is the proof the props reached the trigger rather than being
    // destructured away — a Picker that ignores unknown props leaves this
    // reading `select-trigger`, with no error anywhere.
    const trigger = screen.getByRole('combobox', { name: 'Mode' })
    expect(trigger.getAttribute('data-slot')).toBe('tooltip-trigger')
  })
})
