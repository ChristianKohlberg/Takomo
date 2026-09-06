import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { InlineSection } from './InlineSection'

describe('inline section entry', () => {
  it('accepts markdown heading syntax and clears the line after insertion', () => {
    const insert = vi.fn(() => true)
    render(<InlineSection locale="en" maxLevel={3} onInsert={insert} />)
    const input = screen.getByRole('textbox')
    fireEvent.change(input, { target: { value: '### Payment deadline' } })
    fireEvent.submit(input.closest('form')!)
    expect(insert).toHaveBeenCalledWith(3, 'Payment deadline')
    expect((input as HTMLInputElement).value).toBe('')
  })

  it('keeps a draft when insertion cannot succeed and refuses skipped heading levels', () => {
    const insert = vi.fn(() => false)
    render(<InlineSection locale="en" maxLevel={1} onInsert={insert} />)
    const input = screen.getByRole('textbox')
    fireEvent.change(input, { target: { value: '## No parent' } })
    fireEvent.submit(input.closest('form')!)
    expect(insert).not.toHaveBeenCalled()
    expect((input as HTMLInputElement).value).toBe('## No parent')
    expect(screen.getByRole('alert')).toBeTruthy()
    fireEvent.change(input, { target: { value: 'Draft' } })
    fireEvent.submit(input.closest('form')!)
    expect(insert).toHaveBeenCalledWith(1, 'Draft')
    expect((input as HTMLInputElement).value).toBe('Draft')
  })
})
