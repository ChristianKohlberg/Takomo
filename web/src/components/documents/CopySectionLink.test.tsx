import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { CopySectionLink } from './CopySectionLink'

afterEach(() => { cleanup(); vi.restoreAllMocks() })
describe('CopySectionLink', () => {
  it('copies the exact section URL and confirms success', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    const href = 'https://example.test/projects/project/specification?view=document&section=mn-123'
    render(<CopySectionLink href={href} locale="en" />)
    fireEvent.click(screen.getByRole('button', { name: 'Copy section link' }))
    await waitFor(() => expect(screen.getByRole('status').textContent).toBe('Copied'))
    expect(writeText).toHaveBeenCalledWith(href)
    vi.unstubAllGlobals()
  })
  it('offers a selectable link when clipboard access is refused', async () => {
    vi.stubGlobal('navigator', { clipboard: { writeText: vi.fn().mockRejectedValue(new Error('denied')) } })
    render(<CopySectionLink href="https://example.test/section" locale="en" />)
    fireEvent.click(screen.getByRole('button', { name: 'Copy section link' }))
    const input = await screen.findByRole('textbox', { name: 'Copy this link manually' }) as HTMLInputElement
    expect(input.value).toBe('https://example.test/section')
    input.focus()
    expect(input.selectionStart).toBe(0)
    expect(input.selectionEnd).toBe(input.value.length)
    vi.unstubAllGlobals()
  })
})
