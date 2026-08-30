// The double confirm, which is the whole point of this dialog.
//
// Cutting a line is reached by ONE click on a deliberately fat target, so the
// thing worth proving is that one click is not enough: the first press advances
// the question and the second is what cuts. jsdom says nothing about how any of
// it looks, and does not need to.
import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'

import { DetachDialog } from './DetachDialog'

const LABELS = {
  title: 'Cut this line?',
  body: '“{title}” stops hanging off “{parent}”.',
  confirmTitle: 'Once more, then',
  confirmBody: 'This moves “{title}” out of the branch it is read in.',
  carries: '{n} thoughts under it come with it.',
  watching: 'In this map right now: {names}',
  next: 'Continue',
  detach: 'Cut the line',
  cancel: 'Cancel',
}

const edge = { child: { id: 'mn-2', title: 'Per seat' }, parentTitle: 'Pricing' }

describe('DetachDialog', () => {
  it('asks twice before cutting anything', () => {
    const onConfirm = vi.fn()
    render(
      <DetachDialog
        edge={edge}
        carries={3}
        peers={[]}
        onOpenChange={() => {}}
        onConfirm={onConfirm}
        labels={LABELS}
      />,
    )

    expect(screen.getByText('“Per seat” stops hanging off “Pricing”.')).toBeTruthy()
    expect(screen.getByText('3 thoughts under it come with it.')).toBeTruthy()

    fireEvent.click(screen.getByText('Continue'))
    expect(onConfirm).not.toHaveBeenCalled()
    expect(screen.getByText('Once more, then')).toBeTruthy()

    fireEvent.click(screen.getByText('Cut the line'))
    expect(onConfirm).toHaveBeenCalledWith('mn-2')
  })

  it('names the people watching, but only on the second question', () => {
    render(
      <DetachDialog
        edge={edge}
        carries={0}
        peers={['Ada', 'Grace']}
        onOpenChange={() => {}}
        onConfirm={() => {}}
        labels={LABELS}
      />,
    )
    expect(screen.queryByText('In this map right now: Ada, Grace')).toBeNull()
    fireEvent.click(screen.getByText('Continue'))
    expect(screen.getByText('In this map right now: Ada, Grace')).toBeTruthy()
  })

  it('draws nothing when no line is being cut', () => {
    const { container } = render(
      <DetachDialog
        edge={null}
        carries={0}
        peers={[]}
        onOpenChange={() => {}}
        onConfirm={() => {}}
        labels={LABELS}
      />,
    )
    expect(container.textContent).toBe('')
  })
})
