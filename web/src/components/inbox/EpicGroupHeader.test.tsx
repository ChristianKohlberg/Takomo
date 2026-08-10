// The disclosure contract of a grouped inbox.
//
// jsdom has no layout engine, so nothing here can prove the heading LOOKS
// right. What it can prove is the part a keyboard and a screen reader depend
// on: the heading is a real button, it says which way it goes, and clicking it
// asks to go the other way. Those are exactly the properties a clickable <div>
// would have silently lost.
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, screen, cleanup, fireEvent } from '@testing-library/react'
import { EpicGroupHeader } from './EpicGroupHeader'

afterEach(cleanup)

describe('EpicGroupHeader', () => {
  it('announces expanded state, and toggles the other way', () => {
    const onToggle = vi.fn()
    render(
      <EpicGroupHeader title="Billing revamp" count={3} collapsed={false} onToggle={onToggle} />,
    )
    const heading = screen.getByRole('button', { name: /billing revamp/i })
    expect(heading.getAttribute('aria-expanded')).toBe('true')
    fireEvent.click(heading)
    expect(onToggle).toHaveBeenCalledTimes(1)
  })

  it('announces collapsed state', () => {
    render(<EpicGroupHeader title="Billing revamp" count={3} collapsed onToggle={() => {}} />)
    expect(
      screen.getByRole('button', { name: /billing revamp/i }).getAttribute('aria-expanded'),
    ).toBe('false')
  })

  it('shows the count, so a folded group still says how much is inside', () => {
    render(<EpicGroupHeader title="Billing revamp" count={11} collapsed onToggle={() => {}} />)
    expect(screen.getByText('11')).toBeTruthy()
  })
})
