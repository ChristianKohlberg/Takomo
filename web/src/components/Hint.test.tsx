// Two properties of `Hint` that nothing else would notice breaking.
//
// The first is that it renders standalone. `Hint` carries its own
// `TooltipProvider` precisely so a component containing one can be rendered by a
// test or by a design-system consumer with no app around it; move the provider
// back up to the root and Radix throws, which is how this was found the first
// time — 26 NavRail tests at once.
//
// The second is that it never becomes the trigger's accessible NAME. A tooltip
// describes; it does not name. If a future edit reaches for `aria-label` here it
// would silently override the visible text on every labelled control that has a
// hint, which is the "label in name" failure and is invisible to everything
// except a screen reader.
import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import { Hint } from './Hint'

describe('Hint', () => {
  it('renders with no provider around it', () => {
    render(
      <Hint text="Re-run every check">
        <button type="button">Run</button>
      </Hint>,
    )
    expect(screen.getByRole('button', { name: 'Run' })).toBeDefined()
  })

  it('leaves the trigger to name itself', () => {
    render(
      <Hint text="This is a description, not a name">
        <button type="button">Archive</button>
      </Hint>,
    )
    const btn = screen.getByRole('button', { name: 'Archive' })
    expect(btn.getAttribute('aria-label')).toBeNull()
  })

  it('renders the child alone when there is no text to show', () => {
    const { container } = render(
      <Hint text="">
        <button type="button">Bare</button>
      </Hint>,
    )
    expect(screen.getByRole('button', { name: 'Bare' })).toBeDefined()
    // No trigger wrapper attributes at all — an absent hint costs nothing.
    expect(container.querySelector('[data-slot="tooltip-trigger"]')).toBeNull()
  })
})
