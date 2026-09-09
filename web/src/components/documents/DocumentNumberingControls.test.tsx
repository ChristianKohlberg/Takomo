// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it } from 'vitest'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { DocumentNumberingControls, useDocumentNumbering } from './DocumentNumberingControls'
import type { DocumentAppearance } from '@/lib/document-appearance'

beforeEach(() => localStorage.clear())
afterEach(cleanup)
const base: DocumentAppearance = { template: 'balanced', overrides: {} }
function Toolbar({ project = 'one', appearance = base }: { project?: string; appearance?: DocumentAppearance }) {
  const numbering = useDocumentNumbering(project, appearance)
  return <DocumentNumberingControls {...numbering} locale="en" />
}
it('lets every reader toggle numbers, persists per project and restores current defaults', () => {
  const ui = render(<Toolbar />)
  fireEvent.click(screen.getByRole('button', { name: 'Numbers for H1' }))
  expect(screen.getByRole('button', { name: 'Numbers for H1' }).getAttribute('aria-pressed')).toBe('false')
  ui.rerender(<Toolbar project="two" />)
  expect(screen.getByRole('button', { name: 'Numbers for H1' }).getAttribute('aria-pressed')).toBe('true')
  ui.rerender(<Toolbar project="one" appearance={{ ...base, numbering: { h2: false } }} />)
  expect(screen.getByRole('button', { name: 'Numbers for H1' }).getAttribute('aria-pressed')).toBe('false')
  expect(screen.getByRole('button', { name: 'Numbers for H2' }).getAttribute('aria-pressed')).toBe('false')
  fireEvent.click(screen.getByRole('button', { name: 'Use project defaults' }))
  expect(screen.getByRole('button', { name: 'Numbers for H1' }).getAttribute('aria-pressed')).toBe('true')
  expect(screen.getByRole('button', { name: 'Numbers for H2' }).getAttribute('aria-pressed')).toBe('false')
  expect(localStorage.getItem('takomo:document-numbering:one')).toBeNull()
})
it('restores preferences after remount and ignores malformed storage', () => {
  const ui = render(<Toolbar />)
  fireEvent.click(screen.getByRole('button', { name: 'Numbers for H2' }))
  ui.unmount()
  render(<Toolbar />)
  expect(screen.getByRole('button', { name: 'Numbers for H2' }).getAttribute('aria-pressed')).toBe('false')
  cleanup()
  localStorage.setItem('takomo:document-numbering:one', '{bad')
  render(<Toolbar />)
  expect(screen.getByRole('button', { name: 'Numbers for H2' }).getAttribute('aria-pressed')).toBe('true')
})
