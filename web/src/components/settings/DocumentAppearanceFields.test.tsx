// @vitest-environment jsdom
import { useState } from 'react'
import { afterEach, describe, expect, it } from 'vitest'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { DocumentAppearanceFields } from './DocumentAppearanceFields'
import { DOCUMENT_APPEARANCE_STRINGS } from '@/pages/settings/strings'
import type { DocumentAppearance } from '@/lib/document-appearance'

afterEach(cleanup)

function Form({ disabled = false }: { disabled?: boolean }) {
  const [value, setValue] = useState<DocumentAppearance>({ template: 'balanced', overrides: {} })
  return <DocumentAppearanceFields value={value} onChange={setValue} disabled={disabled} labels={DOCUMENT_APPEARANCE_STRINGS.en} />
}

describe('document appearance controls', () => {
  it('switches templates, retains overrides, and resets an individual value', () => {
    render(<Form />)
    const h1 = screen.getByLabelText('H1 size (px)') as HTMLInputElement
    fireEvent.change(h1, { target: { value: '36' } })
    fireEvent.change(screen.getByLabelText('Template'), { target: { value: 'strong' } })
    expect(h1.value).toBe('36')
    expect((screen.getByLabelText('H2 size (px)') as HTMLInputElement).value).toBe('24')
    expect(screen.getByText('Project specification').style.fontSize).toBe('36px')
    fireEvent.click(screen.getByRole('button', { name: 'Reset: H1 size (px)' }))
    expect(h1.value).toBe('32')
    expect(screen.queryByRole('button', { name: 'Reset: H1 size (px)' })).toBeNull()
  })
  it('disables template and numeric controls for read-only projects', () => {
    render(<Form disabled />)
    expect((screen.getByLabelText('Template') as HTMLSelectElement).disabled).toBe(true)
    for (const input of screen.getAllByRole('spinbutton')) expect((input as HTMLInputElement).disabled).toBe(true)
  })
})
