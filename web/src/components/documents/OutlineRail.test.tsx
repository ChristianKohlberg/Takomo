// jsdom can prove nothing about indent, dot size or type weight — that is why
// the model is a pure module with its own tests. What IS testable here is the
// wiring: which rows exist, what a fold hides, and that a folder nothing is
// filed as opens instead of pretending to be a document.
import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { OutlineRail } from './OutlineRail'
import { buildOutline } from '@/lib/document-outline'
import type { Doc } from '@/lib/documents'

function doc(id: string, title: string, path: string): Doc {
  return {
    id,
    project: 'tp',
    title,
    path,
    status: 'draft',
    initiative: null,
    metadata: null,
    version: 1,
    created_by: 'test',
    created_at: '2026-08-23T00:00:00.000Z',
    updated_at: '2026-08-23T00:00:00.000Z',
    archived_at: null,
    bytes: 0,
    updates: 0,
  }
}

const labels = {
  expand: 'Open this section',
  collapse: 'Close this section',
  folded: '{n} sections inside',
  group: 'A folder.',
  archive: 'Archive',
  unarchive: 'Restore',
  archived: 'archived',
  waiting: '{n} proposals waiting',
}

const sections = buildOutline([
  doc('a', 'Payments', ''),
  doc('b', 'API', 'Payments'),
  doc('c', 'Versioning', 'Payments/API'),
  doc('d', 'Loose', 'hand/filed'),
])

describe('OutlineRail', () => {
  it('shows a section number beside every row', () => {
    render(
      <OutlineRail
        sections={sections}
        selected={null}
        onSelect={() => {}}
        collapsed={new Set()}
        onToggle={() => {}}
        labels={labels}
      />,
    )
    expect(screen.getByText('1.1')).toBeTruthy()
    expect(screen.getByText('1.1.1')).toBeTruthy()
    // The folder `Payments/API` and the document `API` are ONE row, not two.
    expect(screen.getAllByText('API')).toHaveLength(1)
  })

  it('hides what a folded section holds, and says how much', () => {
    render(
      <OutlineRail
        sections={sections}
        selected={null}
        onSelect={() => {}}
        collapsed={new Set(['a'])}
        onToggle={() => {}}
        labels={labels}
      />,
    )
    expect(screen.queryByText('API')).toBeNull()
    expect(screen.getByTitle('2 sections inside')).toBeTruthy()
  })

  it('opens a document, and only opens or closes a folder nothing is filed as', () => {
    const onSelect = vi.fn()
    const onToggle = vi.fn()
    render(
      <OutlineRail
        sections={sections}
        selected={null}
        onSelect={onSelect}
        collapsed={new Set()}
        onToggle={onToggle}
        labels={labels}
      />,
    )
    screen.getByText('Loose').click()
    expect(onSelect).toHaveBeenCalledWith('d')

    screen.getByText('hand').click()
    expect(onToggle).toHaveBeenCalledWith('hand')
    expect(onSelect).toHaveBeenCalledTimes(1)
  })

  it('offers archiving only when the page passes a handler', () => {
    const { rerender } = render(
      <OutlineRail
        sections={sections}
        selected={null}
        onSelect={() => {}}
        collapsed={new Set()}
        onToggle={() => {}}
        labels={labels}
      />,
    )
    expect(screen.queryAllByLabelText('Archive')).toHaveLength(0)
    rerender(
      <OutlineRail
        sections={sections}
        selected={null}
        onSelect={() => {}}
        collapsed={new Set()}
        onToggle={() => {}}
        onArchiveToggle={() => {}}
        labels={labels}
      />,
    )
    expect(screen.getAllByLabelText('Archive').length).toBeGreaterThan(0)
  })

  it('marks the open document with its waiting proposals', () => {
    render(
      <OutlineRail
        sections={sections}
        selected="b"
        onSelect={() => {}}
        collapsed={new Set()}
        onToggle={() => {}}
        pending={{ b: 3 }}
        labels={labels}
      />,
    )
    expect(screen.getByTitle('3 proposals waiting').textContent).toBe('3')
  })
})
