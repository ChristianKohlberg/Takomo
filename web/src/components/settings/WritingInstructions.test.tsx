import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { WritingInstructions } from './WritingInstructions'
import { getWritingInstructions, saveWritingInstructions, type WritingInstructions as Settings } from '@/lib/writing-instructions'

vi.mock('@/lib/writing-instructions', async (importOriginal) => ({
  ...await importOriginal<typeof import('@/lib/writing-instructions')>(),
  getWritingInstructions: vi.fn(),
  saveWritingInstructions: vi.fn(),
}))

afterEach(() => { cleanup(); vi.resetAllMocks() })
const saved: Settings = {
  templates: [{ id: 'brief', name: 'Brief', instruction: 'Keep headings short.' }],
  default_id: 'brief',
}

describe('project writing instructions', () => {
  it('starts empty and saves a user-written instruction only for the selected project', async () => {
    vi.mocked(getWritingInstructions).mockResolvedValue({ templates: [], default_id: null })
    vi.mocked(saveWritingInstructions).mockImplementation(async (_token, _project, value) => value)
    render(<WritingInstructions token="token" project="jeans" readOnly={false} lang="en" />)
    await screen.findByText('No writing instructions yet. Add an instruction in your own words.')
    fireEvent.click(screen.getByRole('button', { name: 'Add instruction' }))
    expect((screen.getByLabelText('Instruction') as HTMLTextAreaElement).value).toBe('')
    expect((screen.getByRole('button', { name: 'Save writing instructions' }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Materials' } })
    fireEvent.change(screen.getByLabelText('Instruction'), { target: { value: 'Describe fabric and fit in the body.' } })
    const id = (screen.getByLabelText('Project default') as HTMLSelectElement).options[1]!.value
    fireEvent.change(screen.getByLabelText('Project default'), { target: { value: id } })
    fireEvent.click(screen.getByRole('button', { name: 'Save writing instructions' }))
    await screen.findByText('Writing instructions saved.')
    expect(saveWritingInstructions).toHaveBeenCalledWith('token', 'jeans', {
      templates: [{ id, name: 'Materials', instruction: 'Describe fabric and fit in the body.' }], default_id: id,
    })
  })

  it('clears the default when its instruction is removed and preserves drafts after a failed save', async () => {
    vi.mocked(getWritingInstructions).mockResolvedValue(saved)
    vi.mocked(saveWritingInstructions).mockRejectedValue(new Error('Connection lost'))
    render(<WritingInstructions token="token" project="takomo" readOnly={false} lang="en" />)
    await screen.findByLabelText('Name')
    fireEvent.click(screen.getByRole('button', { name: 'Remove' }))
    expect((screen.getByLabelText('Project default') as HTMLSelectElement).value).toBe('')
    fireEvent.click(screen.getByRole('button', { name: 'Save writing instructions' }))
    await screen.findByRole('alert')
    expect(saveWritingInstructions).toHaveBeenCalledWith('token', 'takomo', { templates: [], default_id: null })
    expect(screen.queryByLabelText('Instruction')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Discard changes' }))
    expect((screen.getByLabelText('Instruction') as HTMLTextAreaElement).value).toBe('Keep headings short.')
  })

  it('does not carry drafts or delayed responses into a different project', async () => {
    let resolveFirst!: (settings: Settings) => void
    vi.mocked(getWritingInstructions).mockImplementation((_token, project) => project === 'first'
      ? new Promise((resolve) => { resolveFirst = resolve })
      : Promise.resolve({ templates: [], default_id: null }))
    const { rerender } = render(<WritingInstructions token="token" project="first" readOnly={false} lang="en" />)
    rerender(<WritingInstructions token="token" project="second" readOnly={false} lang="en" />)
    await screen.findByText('No writing instructions yet. Add an instruction in your own words.')
    resolveFirst(saved)
    await waitFor(() => expect(screen.queryByDisplayValue('Brief')).toBeNull())
    expect(saveWritingInstructions).not.toHaveBeenCalled()
  })

  it('shows saved instructions but prevents editing for readers or archived projects', async () => {
    vi.mocked(getWritingInstructions).mockResolvedValue(saved)
    render(<WritingInstructions token="reader" project="takomo" readOnly lang="en" />)
    await screen.findByLabelText('Name')
    expect((screen.getByLabelText('Project default') as HTMLSelectElement).disabled).toBe(true)
    expect(screen.getByLabelText('Instruction').closest('fieldset')?.disabled).toBe(true)
    expect(screen.queryByRole('button', { name: 'Save writing instructions' })).toBeNull()
  })
})
