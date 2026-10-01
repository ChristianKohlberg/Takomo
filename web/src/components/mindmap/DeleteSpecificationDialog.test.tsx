// Deleting a project's specification takes two steps and a typed confirmation,
// the way resetting a document in Settings does.
import { describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen } from '@testing-library/react'

import { DeleteSpecificationDialog, confirmsSpecification } from './DeleteSpecificationDialog'

const LABELS = {
  first: 'Delete this specification? (1 of 2)',
  second: 'Confirm deletion (2 of 2)',
  warning: 'Every section goes.',
  irreversible: 'Cannot be undone.',
  continue: 'Continue',
  typeToConfirm: 'Type its ID or title',
  final: 'Delete specification',
  busy: 'Deleting…',
  cancel: 'Cancel',
}
const TARGET = { id: 'mm-abc123', title: 'Payments rebuild', project: 'tp' }

describe('confirmsSpecification', () => {
  it('accepts the id or the title, and nothing else', () => {
    expect(confirmsSpecification('mm-abc123', TARGET)).toBe(true)
    expect(confirmsSpecification('  Payments rebuild ', TARGET)).toBe(true)
    expect(confirmsSpecification('payments rebuild', TARGET)).toBe(false)
    expect(confirmsSpecification('mm-abc', TARGET)).toBe(false)
    expect(confirmsSpecification('', { id: 'mm-x', title: '' })).toBe(false)
  })
})

describe('DeleteSpecificationDialog', () => {
  it('only deletes after the warning, the second step and the typed id', async () => {
    const onConfirm = vi.fn().mockResolvedValue(undefined)
    const onOpenChange = vi.fn()
    render(<DeleteSpecificationDialog target={TARGET} onOpenChange={onOpenChange} onConfirm={onConfirm} labels={LABELS} />)
    expect(screen.getByText(LABELS.first)).toBeTruthy()
    expect(screen.queryByRole('button', { name: LABELS.final })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: LABELS.continue }))

    const final = screen.getByRole('button', { name: LABELS.final }) as HTMLButtonElement
    expect(final.disabled).toBe(true)
    fireEvent.change(screen.getByLabelText(LABELS.typeToConfirm), { target: { value: 'mm-wrong' } })
    expect(final.disabled).toBe(true)
    fireEvent.click(final)
    expect(onConfirm).not.toHaveBeenCalled()

    fireEvent.change(screen.getByLabelText(LABELS.typeToConfirm), { target: { value: 'Payments rebuild' } })
    expect(final.disabled).toBe(false)
    await act(async () => {
      fireEvent.click(final)
    })
    expect(onConfirm).toHaveBeenCalledWith('mm-abc123')
    expect(onOpenChange).toHaveBeenCalledWith(false)
  })

  it('shows a refusal and stays open', async () => {
    const onConfirm = vi.fn().mockRejectedValue(new Error('Admin scope required'))
    const onOpenChange = vi.fn()
    render(<DeleteSpecificationDialog target={TARGET} onOpenChange={onOpenChange} onConfirm={onConfirm} labels={LABELS} />)
    fireEvent.click(screen.getByRole('button', { name: LABELS.continue }))
    fireEvent.change(screen.getByLabelText(LABELS.typeToConfirm), { target: { value: 'mm-abc123' } })
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: LABELS.final }))
    })
    expect(screen.getByRole('alert').textContent).toBe('Admin scope required')
    expect(onOpenChange).not.toHaveBeenCalledWith(false)
  })

  it('renders nothing without a target', () => {
    const { container } = render(
      <DeleteSpecificationDialog target={null} onOpenChange={vi.fn()} onConfirm={vi.fn()} labels={LABELS} />,
    )
    expect(container.textContent).toBe('')
  })
})
