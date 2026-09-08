import { fireEvent, render, screen } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import { DocumentActions } from './DocumentActions'

function panel(container: HTMLElement) {
  return container.querySelector('.document-more-tools')?.getAttribute('data-open') === 'true'
}

it('keeps the overflow panel open for popover triggers and repeated undo, closing for one-shot actions', () => {
  const onTextUndo = vi.fn(), onFind = vi.fn()
  const { container } = render(<DocumentActions locale="en" findOpen={false} onFind={onFind} canWrite textUndo textRedo={false} moveUndo={false} moveRedo={false}
    onTextUndo={onTextUndo} onTextRedo={vi.fn()} onMoveUndo={vi.fn()} onMoveRedo={vi.fn()}>
    <button type="button" aria-haspopup="dialog" aria-label="Insert section reference">Link</button>
  </DocumentActions>)
  expect(panel(container)).toBe(false)
  fireEvent.click(screen.getByRole('button', { name: /Tools/ }))
  expect(panel(container)).toBe(true)
  fireEvent.click(screen.getByRole('button', { name: 'Insert section reference' }))
  expect(panel(container)).toBe(true)
  fireEvent.click(screen.getByRole('button', { name: 'Undo section text' }))
  fireEvent.click(screen.getByRole('button', { name: 'Undo section text' }))
  expect(onTextUndo).toHaveBeenCalledTimes(2)
  expect(panel(container)).toBe(true)
  fireEvent.click(screen.getByRole('button', { name: 'Find in document' }))
  expect(onFind).toHaveBeenCalled()
  expect(panel(container)).toBe(false)
})
