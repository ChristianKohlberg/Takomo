// The one place a thought is read in full and the one place its fields are
// changed. jsdom has no layout engine, so nothing here says the dialog is drawn
// correctly; what it says is that everything the expanded canvas card used to
// show is still reachable here, that the TITLE is a heading rather than a field
// (it is typed on the node), that a field commits when you leave it rather than
// on every keystroke, and that a read-only token gets a dialog it cannot type
// into.
import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'

import { NodeDialog, type NodeDialogLabels } from './NodeDialog'
import type { MapNode, Relationship } from '@/lib/mindmap-doc'

const LABELS: NodeDialogLabels = {
  heading: '“{title}”',
  subtitle: 'Everything this thought holds.',
  origin: 'Written by',
  originHuman: 'person',
  originAgent: 'agent',
  promoted: 'became',
  attachments: 'Attachments ({n})',
  noAttachments: 'Nothing attached to this thought yet.',
  openAttachments: 'Attachments…',
  notes: 'Notes',
  notesHint: 'The long form.',
  notesCount: '{n} of {max} characters',
  kind: 'Kind',
  shape: 'Shape',
  color: 'Colour',
  colorNone: 'No colour',
  edgeLabel: 'Edge label',
  edgeLabelHint: 'Names the line to its parent',
  reviewed: 'A person has looked at this',
  relations: 'Relations',
  removeRelation: 'Remove relation',
  noRelations: 'No relations from this thought.',
  close: 'Close',
  readOnly: 'This token cannot change anything here.',
  kinds: {
    thought: 'Thought',
    question: 'Question',
    decision: 'Decision',
    screen: 'Screen',
    component: 'Component',
  },
  shapes: { rounded: 'Rounded', square: 'Square', pill: 'Pill' },
  question: 'open question',
  answer: 'Your answer',
  answerHint: 'In your own words.',
  answerAction: '⏎ Answer',
  answerAbout: 'Goes into the notes on “{title}”.',
  answerAlone: 'The answer becomes its own notes.',
}

const node = (over: Partial<MapNode> = {}): MapNode => ({
  id: 'mn-1',
  parent: null,
  order: 'a0',
  title: 'Pricing',
  notes: 'the long form',
  at: null,
  edge_label: '',
  kind: 'thought',
  origin: 'human',
  reviewed: false,
  icons: [],
  color: '',
  shape: 'rounded',
  attachments: [],
  promoted: null,
  created_by: 'ada',
  created_at: 0,
  updated_at: 0,
  position: 0,
  ...over,
})

function mount(over: Partial<Parameters<typeof NodeDialog>[0]> = {}) {
  const props = {
    node: node(),
    canWrite: true,
    relations: [] as Relationship[],
    titleOf: new Map<string, string>(),
    onOpenChange: vi.fn(),
    onOpenAttachments: vi.fn(),
    onNotes: vi.fn(),
    onFields: vi.fn(),
    onRemoveRelation: vi.fn(),
    onAnswer: vi.fn(),
    labels: LABELS,
    ...over,
  }
  render(<NodeDialog {...props} />)
  return props
}

describe('NodeDialog', () => {
  it('carries every field the card used to edit inline', () => {
    mount({ node: node({ edge_label: 'costs' }) })
    expect((screen.getByLabelText('Notes') as HTMLTextAreaElement).value).toBe('the long form')
    expect(screen.getByLabelText('Kind')).toBeTruthy()
    expect(screen.getByLabelText('Shape')).toBeTruthy()
    expect((screen.getByLabelText('Edge label') as HTMLInputElement).value).toBe('costs')
    expect(screen.getByLabelText('No colour')).toBeTruthy()
    expect(screen.getByLabelText('A person has looked at this')).toBeTruthy()
  })

  it('shows the title as its heading and offers no way to type one', () => {
    // The title is typed on the NODE, with the inline caret. Two ways to edit one
    // field is the trap; there is exactly one.
    mount()
    expect(screen.getByText('“Pricing”')).toBeTruthy()
    expect(screen.queryByLabelText('Title')).toBeNull()
  })

  it('opens with the caret in the notes', () => {
    // The notes are what somebody came here to write; everything else on this
    // surface is a control rather than a caret.
    mount()
    expect(document.activeElement).toBe(screen.getByLabelText('Notes'))
  })

  it('reads out what the expanded card used to, now that there is no expanded card', () => {
    mount({
      node: node({
        origin: 'agent',
        promoted: { kind: 'epic', id: 'tk-9' },
        attachments: [{ id: 'ma-1', kind: 'pdf', name: 'spec.pdf', gist: 'the rules', ref: '' }],
      }),
    })
    expect(screen.getByText(/Written by agent/)).toBeTruthy()
    expect(screen.getByText(/became epic tk-9/)).toBeTruthy()
    expect(screen.getByText('Attachments (1)')).toBeTruthy()
    expect(screen.getByText(/spec\.pdf/)).toBeTruthy()
  })

  it('reads attachments here and hands over to the manager to change one', () => {
    const props = mount()
    expect(screen.getByText('Nothing attached to this thought yet.')).toBeTruthy()
    fireEvent.click(screen.getByText('Attachments…'))
    expect(props.onOpenAttachments).toHaveBeenCalledWith('mn-1')
  })

  it('writes notes when the field is left, not while it is typed in', () => {
    const props = mount()
    const notes = screen.getByLabelText('Notes')
    fireEvent.change(notes, { target: { value: 'a longer thought' } })
    expect(props.onNotes).not.toHaveBeenCalled()
    fireEvent.blur(notes)
    expect(props.onNotes).toHaveBeenCalledWith('mn-1', 'a longer thought')
  })

  it('commits what is still in a field when the dialog closes', () => {
    // There is no save button here for the reason there is none on /documents:
    // the honest question on a shared document is whether you are connected.
    const props = mount()
    fireEvent.change(screen.getByLabelText('Notes'), { target: { value: 'kept' } })
    fireEvent.click(screen.getByText('Close'))
    expect(props.onNotes).toHaveBeenCalledWith('mn-1', 'kept')
    expect(props.onOpenChange).toHaveBeenCalledWith(false)
  })

  it('sets each scalar field on its own key, so two people do not clobber each other', () => {
    // Separate keys in the document, never one style blob: a blob merges as a
    // whole, so recolouring and re-labelling at the same time would lose one.
    const props = mount()
    fireEvent.change(screen.getByLabelText('Kind'), { target: { value: 'decision' } })
    expect(props.onFields).toHaveBeenNthCalledWith(1, 'mn-1', { kind: 'decision' })
    fireEvent.change(screen.getByLabelText('Shape'), { target: { value: 'pill' } })
    expect(props.onFields).toHaveBeenNthCalledWith(2, 'mn-1', { shape: 'pill' })
    fireEvent.click(screen.getByLabelText('#dbeafe'))
    expect(props.onFields).toHaveBeenNthCalledWith(3, 'mn-1', { color: '#dbeafe' })
    fireEvent.click(screen.getByLabelText('A person has looked at this'))
    expect(props.onFields).toHaveBeenNthCalledWith(4, 'mn-1', { reviewed: true })
  })

  it('lists the relations touching the node and removes the one asked for', () => {
    const props = mount({
      relations: [{ id: 'mr-1', from: 'mn-1', to: 'mn-2', label: 'blocks' }],
      titleOf: new Map([['mn-2', 'Billing']]),
    })
    expect(screen.getByText(/Billing/)).toBeTruthy()
    fireEvent.click(screen.getByLabelText('Remove relation — Billing'))
    expect(props.onRemoveRelation).toHaveBeenCalledWith('mr-1')
  })

  it('answers a question in a person’s own words, with no model in the path', () => {
    const props = mount({
      node: node({ kind: 'question', title: 'Which tier?' }),
      relations: [{ id: 'mr-1', from: 'mn-1', to: 'mn-2', label: '' }],
      titleOf: new Map([['mn-2', 'Pricing']]),
    })
    expect(screen.getByText('Goes into the notes on “Pricing”.')).toBeTruthy()
    fireEvent.change(screen.getByLabelText('Your answer'), { target: { value: 'the middle one' } })
    fireEvent.click(screen.getByText('⏎ Answer'))
    expect(props.onAnswer).toHaveBeenCalledWith('mn-1', 'the middle one')
  })

  it('gives a read-only token the whole thought and nothing to type into', () => {
    mount({
      canWrite: false,
      relations: [{ id: 'mr-1', from: 'mn-1', to: 'mn-2', label: '' }],
      titleOf: new Map([['mn-2', 'Billing']]),
    })
    expect(screen.getByText('“Pricing”')).toBeTruthy()
    for (const field of ['Notes', 'Kind', 'Shape', 'Edge label']) {
      expect((screen.getByLabelText(field) as HTMLInputElement).disabled).toBe(true)
    }
    expect((screen.getByLabelText('No colour') as HTMLButtonElement).disabled).toBe(true)
    expect((screen.getByLabelText('A person has looked at this') as HTMLInputElement).disabled).toBe(
      true,
    )
    expect(screen.queryByLabelText('Remove relation — Billing')).toBeNull()
    expect(screen.getByText('This token cannot change anything here.')).toBeTruthy()
  })
})
