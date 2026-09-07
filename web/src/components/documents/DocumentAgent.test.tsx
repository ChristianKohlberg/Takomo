import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { DocumentAgent } from './DocumentAgent'
import { getDocumentConversation, sendDocumentMessage } from '@/lib/document-conversation'

vi.mock('@/lib/document-conversation', () => ({ getDocumentConversation: vi.fn(), sendDocumentMessage: vi.fn(), setDocumentPins: vi.fn() }))
const props = { token: 'token', project: 'project', map: 'map', lang: 'en' as const, canAsk: true, selected: 'one', onError: vi.fn(), nodes: [{ id: 'one', title: 'First', parent: null, position: 0, order: 'a' }] }
beforeEach(() => { vi.clearAllMocks(); vi.mocked(getDocumentConversation).mockResolvedValue({ conversation: null, messages: [], jobs: [] }) })
function command(editor: HTMLElement) { editor.focus(); fireEvent.keyDown(editor, { key: 'k', ctrlKey: true }) }

describe('document command menu', () => {
  it('opens from the editor, prefills selected scope without sending, and restores focus on Escape', async () => {
    const ui = render(<><div contentEditable suppressContentEditableWarning tabIndex={0}>Document text</div><DocumentAgent {...props} /></>)
    const editor = ui.getByText('Document text')
    command(editor)
    fireEvent.click(await screen.findByRole('option', { name: 'Draft tests' }))
    expect((await screen.findByRole('checkbox', { name: 'First' }) as HTMLInputElement).checked).toBe(true)
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toContain('acceptance tests')
    expect(sendDocumentMessage).not.toHaveBeenCalled()
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Escape' })
    await waitFor(() => expect(document.activeElement).toBe(editor))
  })

  it('resumes an unsent draft, action and scope through Discuss with Codex', async () => {
    render(<DocumentAgent {...props} />)
    const button = screen.getByRole('button', { name: 'Discuss with Codex' })
    command(button)
    fireEvent.click(await screen.findByRole('option', { name: 'Draft questions' }))
    fireEvent.change(await screen.findByRole('textbox'), { target: { value: 'My unfinished question' } })
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Escape' })
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    command(button)
    fireEvent.click(await screen.findByRole('option', { name: 'Discuss with Codex' }))
    expect((await screen.findByRole('textbox') as HTMLTextAreaElement).value).toBe('My unfinished question')
    expect((screen.getByRole('radio', { name: 'My selection' }) as HTMLInputElement).checked).toBe(true)
    expect((screen.getByRole('combobox', { name: 'Action' }) as HTMLSelectElement).value).toBe('draft_questions')
  })

  it('does not intercept shortcuts while another modal is open', () => {
    render(<><div role="dialog"><input aria-label="Other dialog" /></div><DocumentAgent {...props} /></>)
    command(screen.getByRole('textbox', { name: 'Other dialog' }))
    expect(screen.queryByRole('combobox')).toBeNull()
    expect(getDocumentConversation).not.toHaveBeenCalled()
  })
})
