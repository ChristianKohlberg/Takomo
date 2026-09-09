import { fireEvent, render, screen } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import { DocumentActions } from './DocumentActions'

function panel(container: HTMLElement) {
  return container.querySelector('.document-more-tools')?.getAttribute('data-open') === 'true'
}

it('keeps the overflow panel open for popover triggers and repeated undo, closing for one-shot actions', () => {
  const onUndo = vi.fn(), onAction = vi.fn()
  const { container } = render(<DocumentActions locale="en" canWrite canUndo canRedo={false} onUndo={onUndo} onRedo={vi.fn()}>
    <button type="button" onClick={onAction}>One-shot action</button>
    <button type="button" aria-haspopup="dialog" aria-label="Insert section reference">Link</button>
  </DocumentActions>)
  expect(panel(container)).toBe(false)
  fireEvent.click(screen.getByRole('button', { name: /Tools/ }))
  expect(panel(container)).toBe(true)
  fireEvent.click(screen.getByRole('button', { name: 'Insert section reference' }))
  expect(panel(container)).toBe(true)
  fireEvent.click(screen.getByRole('button', { name: 'Undo' }))
  fireEvent.click(screen.getByRole('button', { name: 'Undo' }))
  expect(onUndo).toHaveBeenCalledTimes(2)
  expect(panel(container)).toBe(true)
  fireEvent.click(screen.getByRole('button', { name: 'One-shot action' }))
  expect(onAction).toHaveBeenCalled()
  expect(panel(container)).toBe(false)
})
