// The one behaviour the swap from `<input type="checkbox">` could have quietly
// taken away.
//
// Every filter toggle on /board and /inbox is written as a wrapping label:
//
//   <label><Checkbox checked={x} onCheckedChange={…} /> Group by epic</label>
//
// With a native input, clicking the WORDS toggles the box — that is most of the
// hit area, and on a phone it is nearly all of it. A Radix Checkbox is a
// `<button role="checkbox">`, not an input, so this only keeps working because
// `button` is a labelable element and a label's labeled control is its first
// labelable descendant. That is a spec detail two layers down from the code, and
// nothing else here would notice if a future primitive rendered a `<div>`
// instead — the checkbox would still toggle when clicked directly, and only the
// label text would go dead.
import { describe, expect, it, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import { Checkbox } from './checkbox'

describe('Checkbox', () => {
  it('toggles when the wrapping label text is clicked, not just the box', () => {
    const onCheckedChange = vi.fn()
    render(
      <label>
        <Checkbox checked={false} onCheckedChange={onCheckedChange} />
        Group by epic
      </label>,
    )

    fireEvent.click(screen.getByText('Group by epic'))
    expect(onCheckedChange).toHaveBeenCalledWith(true)
  })

  it('reports its state to assistive tech as a checkbox', () => {
    render(<Checkbox checked aria-label="Mine only" onCheckedChange={() => {}} />)
    const box = screen.getByRole('checkbox', { name: 'Mine only' })
    expect(box.getAttribute('aria-checked')).toBe('true')
  })
})
