// jsdom can prove nothing about indent, dot size or type weight — that is why
// the model is a pure module with its own tests. What IS testable here is the
// wiring: which rows exist, what a fold hides, and that a section's standing is
// readable rather than only coloured.
import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { OutlineRail } from './OutlineRail'
import { planSections, type PlanNode } from '@/lib/plan-sections'

function node(id: string, parent: string | null, title: string, position = 0): PlanNode {
  return { id, parent, title, order: `a${position}`, position }
}

const labels = {
  outline: 'Outline',
  expand: 'Open this section',
  collapse: 'Close this section',
  folded: '{n} sections inside',
  untitled: 'Untitled section',
  standingConfirmed: 'agreed',
  standingChanged: 'changed since',
  standingUnseen: 'unread',
  pending: '{n} proposals waiting for a decision',
}

const sections = planSections([
  node('a', null, 'Payments rebuild'),
  node('b', 'a', 'API'),
  node('c', 'b', 'Versioning'),
  node('d', null, 'Loose', 1),
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

  it('selects the section a row names', () => {
    const onSelect = vi.fn()
    render(
      <OutlineRail
        sections={sections}
        selected={null}
        onSelect={onSelect}
        collapsed={new Set()}
        onToggle={() => {}}
        labels={labels}
      />,
    )
    screen.getByText('Loose').click()
    expect(onSelect).toHaveBeenCalledWith('d')
  })

  it('names a section nobody has titled yet rather than leaving a blank row', () => {
    render(
      <OutlineRail
        sections={planSections([node('a', null, '   ')])}
        selected={null}
        onSelect={() => {}}
        collapsed={new Set()}
        onToggle={() => {}}
        labels={labels}
      />,
    )
    expect(screen.getByText('Untitled section')).toBeTruthy()
  })

  it('says where a section stands in words, not only in colour', () => {
    render(
      <OutlineRail
        sections={sections}
        selected="b"
        onSelect={() => {}}
        collapsed={new Set()}
        onToggle={() => {}}
        standing={{ a: 'confirmed', b: 'changed' }}
        labels={labels}
      />,
    )
    expect(screen.getByTitle('agreed')).toBeTruthy()
    expect(screen.getByTitle('changed since')).toBeTruthy()
    // A section with no history at all carries no mark, rather than a wrong one.
    expect(screen.queryByTitle('unread')).toBeNull()
  })

  it('marks the rows an agent is waiting on, and rolls a folded branch up', () => {
    const { rerender } = render(
      <OutlineRail
        sections={sections}
        selected={null}
        onSelect={() => {}}
        collapsed={new Set()}
        onToggle={() => {}}
        pending={{ c: 2 }}
        labels={labels}
      />,
    )
    expect(screen.getByTitle('2 proposals waiting for a decision')).toBeTruthy()

    // Folded, the branch reports what is waiting INSIDE it: folding is not a
    // decision to stop caring what an agent offered in there.
    rerender(
      <OutlineRail
        sections={sections}
        selected={null}
        onSelect={() => {}}
        collapsed={new Set(['a'])}
        onToggle={() => {}}
        pending={{ c: 2 }}
        labels={labels}
      />,
    )
    expect(screen.queryByText('Versioning')).toBeNull()
    expect(screen.getByTitle('2 proposals waiting for a decision')).toBeTruthy()
  })
})
