import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { MoveSectionDialog } from './MoveSectionDialog'
import type { PlanSection } from '@/lib/plan-sections'

const sections: PlanSection[] = [
  { key: 'a', title: 'Billing', number: '1', depth: 0, children: [{ key: 'child', title: 'Invoices', number: '1.1', depth: 1, children: [] }] },
  { key: 'b', title: 'Reports', number: '2', depth: 0, children: [] },
]

describe('Move section dialog', () => {
  it('previews a chosen hierarchy, excluding descendants, before committing', () => {
    const onMove = vi.fn(() => ({ ok: true as const }))
    const onClose = vi.fn()
    render(<MoveSectionDialog sections={sections} sectionKey="a" lang="en" onMove={onMove} onClose={onClose} />)
    expect(screen.queryByRole('option', { name: '1.1 Invoices' })).toBeNull()
    fireEvent.change(screen.getByLabelText('Destination section'), { target: { value: 'b' } })
    fireEvent.change(screen.getByLabelText('Position'), { target: { value: 'child' } })
    expect(screen.getByText(/New level/).textContent).toContain('New level: 2')
    expect(onMove).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Move' }))
    expect(onMove).toHaveBeenCalledWith('a', 'b', 'child')
    expect(onClose).toHaveBeenCalledOnce()
  })
  it('preserves the dialog when the destination disappears and supports cancellation', () => {
    const onClose = vi.fn()
    render(<MoveSectionDialog sections={sections} sectionKey="a" lang="en" onMove={() => ({ ok: false, error: 'missing' })} onClose={onClose} />)
    fireEvent.change(screen.getByLabelText('Destination section'), { target: { value: 'b' } })
    fireEvent.click(screen.getByRole('button', { name: 'Move' }))
    expect(screen.getByRole('alert').textContent).toContain('content is preserved')
    expect(onClose).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(onClose).toHaveBeenCalledOnce()
  })
})
