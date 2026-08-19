// The explorer's waiting badges, minus what jsdom cannot see (layout — see
// web/README.md).
//
// The badges are the only thing on a row that says a document wants something
// from you, so what matters is that a quiet document stays quiet — a badge that
// appears on everything is one readers stop seeing — and that a document with
// something waiting says which kind, since a decision and a question want
// different amounts of the reader's day.
import { describe, it, expect, afterEach, vi } from 'vitest'
import { render, screen, cleanup, fireEvent } from '@testing-library/react'
import { Explorer } from './Explorer'
import { buildTree } from '@/lib/initiative-tree'
import type { Initiative, Rollup } from '@/lib/initiatives'

afterEach(cleanup)

const labels = {
  empty: 'No documents yet',
  emptyHint: 'Create one, or let an agent open it over MCP.',
  toggle: 'Toggle folder',
  unfiled: 'Unfiled',
  // Shaped like the page's own labels, singular included — the badges are read
  // aloud, so "1 open notes" is a defect rather than a rough edge.
  waitingNotes: (n: number) => (n === 1 ? '1 open note' : `${n} open notes`),
  waitingAmendments: (n: number) =>
    n === 1 ? '1 suggested change awaiting a decision' : `${n} suggested changes awaiting a decision`,
  menu: 'Document actions',
  rename: 'Rename…',
}

function doc(id: string, title: string, rollup?: Rollup): Initiative {
  return { id, project: 'demo', title, status: 'open', rollup }
}

function show(items: Initiative[], onRename?: (i: Initiative) => void) {
  render(
    <Explorer
      root={buildTree(items)}
      selectedId={null}
      expanded={new Set()}
      onToggle={() => {}}
      onSelect={() => {}}
      onRename={onRename}
      labels={labels}
    />,
  )
}

describe('Explorer waiting badges', () => {
  it('says nothing about a document with nothing waiting', () => {
    show([doc('ini-a', 'Quiet', { entries: 12, open_notes: 0, pending_amendments: 0 })])
    expect(screen.getByRole('treeitem', { name: /Quiet/ })).toBeTruthy()
    expect(screen.queryByLabelText(/open notes/)).toBeNull()
    expect(screen.queryByLabelText(/awaiting a decision/)).toBeNull()
  })

  it('distinguishes an unanswered question from an undecided rewrite', () => {
    show([doc('ini-b', 'Busy', { entries: 30, open_notes: 3, pending_amendments: 2 })])
    expect(screen.getByLabelText('3 open notes').textContent).toBe('3')
    expect(screen.getByLabelText('2 suggested changes awaiting a decision').textContent).toBe('2')
  })

  it('shows only the kind that is actually waiting', () => {
    show([doc('ini-c', 'Notes only', { open_notes: 1, pending_amendments: 0 })])
    expect(screen.getByLabelText('1 open note')).toBeTruthy()
    expect(screen.queryByLabelText(/awaiting a decision/)).toBeNull()
  })

  // A server that predates the counts sends neither. Reading that as "quiet" is
  // the honest degradation: inventing attention from a missing field would send
  // readers into documents with nothing in them.
  it('treats a rollup without the counts as quiet', () => {
    show([doc('ini-d', 'Old server', { entries: 4 })])
    expect(screen.queryByLabelText(/open notes/)).toBeNull()
    show([doc('ini-e', 'No rollup at all')])
    expect(screen.queryByLabelText(/open notes/)).toBeNull()
  })
})

// Renaming from the tree, which is where a wrong name is noticed: comparing
// names is what a tree is for, and before this the only way to fix one was to
// open that document and lose the one you were reading.
describe('Explorer rename menu', () => {
  it('opens a menu naming the document that was right-clicked', () => {
    const onRename = vi.fn()
    show([doc('ini-a', 'Billing'), doc('ini-b', 'Onboarding')], onRename)
    expect(screen.queryByRole('menu')).toBeNull()

    fireEvent.contextMenu(screen.getByRole('treeitem', { name: /Onboarding/ }))
    const menu = screen.getByRole('menu', { name: labels.menu })
    expect(menu.textContent).toContain('Onboarding')

    fireEvent.click(screen.getByRole('menuitem', { name: labels.rename }))
    expect(onRename).toHaveBeenCalledTimes(1)
    expect(onRename.mock.calls[0]![0].id).toBe('ini-b')
    // The menu is spent once it has acted; leaving it open over a dialog would
    // put two things on screen competing for the same Escape.
    expect(screen.queryByRole('menu')).toBeNull()
  })

  it('closes on Escape without renaming anything', () => {
    const onRename = vi.fn()
    show([doc('ini-a', 'Billing')], onRename)
    fireEvent.contextMenu(screen.getByRole('treeitem', { name: /Billing/ }))
    expect(screen.getByRole('menu')).toBeTruthy()
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(screen.queryByRole('menu')).toBeNull()
    expect(onRename).not.toHaveBeenCalled()
  })

  // A reader who cannot write gets the browser's own menu back: an entry whose
  // save would be refused is worse than no entry.
  it('leaves right-click alone when renaming is not offered', () => {
    show([doc('ini-a', 'Billing')])
    fireEvent.contextMenu(screen.getByRole('treeitem', { name: /Billing/ }))
    expect(screen.queryByRole('menu')).toBeNull()
  })
})
