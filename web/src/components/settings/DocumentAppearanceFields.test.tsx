// @vitest-environment jsdom
import { useState } from 'react'
import { afterEach, describe, expect, it } from 'vitest'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { DocumentAppearanceFields } from './DocumentAppearanceFields'
import { DOCUMENT_APPEARANCE_STRINGS } from '@/pages/settings/strings'
import { validDocumentAppearance, type DocumentAppearance } from '@/lib/document-appearance'

afterEach(cleanup)

function Form({ disabled = false, onValue }: { disabled?: boolean; onValue?: (value: DocumentAppearance) => void }) {
  const [value, setValue] = useState<DocumentAppearance>({ template: 'balanced', overrides: {} })
  const change = (next: DocumentAppearance) => {
    setValue(next)
    onValue?.(next)
  }
  return <DocumentAppearanceFields value={value} onChange={change} disabled={disabled} labels={DOCUMENT_APPEARANCE_STRINGS.en} />
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
  it('lets a value be cleared and retyped without snapping back to the template', () => {
    let latest: DocumentAppearance | undefined
    render(<Form onValue={(v) => { latest = v }} />)
    const h2 = screen.getByLabelText('H2 size (px)') as HTMLInputElement
    fireEvent.change(h2, { target: { value: '' } })
    expect(h2.value).toBe('')
    expect(h2.getAttribute('aria-invalid')).toBe('true')
    expect(latest && validDocumentAppearance(latest)).toBe(false)
    expect(screen.getByText('Document structure').style.fontSize).toBe('22px')
    fireEvent.change(h2, { target: { value: '26' } })
    expect(h2.value).toBe('26')
    expect(h2.getAttribute('aria-invalid')).toBeNull()
    expect(latest?.overrides.h2_size).toBe(26)
    expect(screen.getByText('Document structure').style.fontSize).toBe('26px')
  })
  it('restores the template value when a field is left empty on blur', () => {
    let latest: DocumentAppearance | undefined
    render(<Form onValue={(v) => { latest = v }} />)
    const h3 = screen.getByLabelText('H3 size (px)') as HTMLInputElement
    fireEvent.change(h3, { target: { value: '21' } })
    fireEvent.change(h3, { target: { value: '' } })
    expect(h3.value).toBe('')
    fireEvent.blur(h3)
    expect(h3.value).toBe('18')
    expect(latest?.overrides.h3_size).toBeUndefined()
    expect(screen.queryByRole('button', { name: 'Reset: H3 size (px)' })).toBeNull()
  })
  it('resets a cleared field to the template immediately', () => {
    render(<Form />)
    const body = screen.getByLabelText('Body size (px)') as HTMLInputElement
    fireEvent.change(body, { target: { value: '' } })
    fireEvent.click(screen.getByRole('button', { name: 'Reset: Body size (px)' }))
    expect(body.value).toBe('16')
    expect(body.getAttribute('aria-invalid')).toBeNull()
  })
  it('disables template and numeric controls for read-only projects', () => {
    render(<Form disabled />)
    expect((screen.getByLabelText('Template') as HTMLSelectElement).disabled).toBe(true)
    for (const input of screen.getAllByRole('spinbutton')) expect((input as HTMLInputElement).disabled).toBe(true)
  })
})
