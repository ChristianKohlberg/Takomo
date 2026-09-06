import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import { EpicsView } from './EpicsView'
import { EPICS_STR } from './epics-strings'
import { applyEpicsGrid, DEFAULT_FILTERS, needsAttention } from './epicsGrid'
import type { RoadmapEpic } from '@/lib/roadmap'

afterEach(() => { cleanup(); vi.restoreAllMocks() })
const base: RoadmapEpic = {
  id: 'demo-a', title: 'Billing v2', state: 'ready', state_category: 'todo', priority: 'normal',
  total: 12, done: 8, percent: 67, ready: 2, backlog: 2, awaiting_answer: 0,
  initiatives: ['ini-billing'], claim: null, flags: [], last_activity_at: '2026-09-06T09:00:00Z',
}
const claim = { holder: 'agent:builder', held_since: null, held_for_seconds: 90000,
  indefinite: true, expires_at: null, last_activity_at: '2026-09-05T09:00:00Z', idle_seconds: 90000 }
const labels = EPICS_STR.en
function view(epics: RoadmapEpic[], extra: Partial<React.ComponentProps<typeof EpicsView>> = {}) {
  const onOpen = vi.fn()
  render(<EpicsView epics={epics} laneTitles={{ 'ini-billing': 'Billing' }} onOpen={onOpen} labels={labels} {...extra} />)
  return onOpen
}
function choose(label: string, option: string) {
  fireEvent.keyDown(screen.getByLabelText(label), { key: 'ArrowDown' })
  fireEvent.click(screen.getByRole('option', { name: option }))
}

describe('epic list ordering and filters', () => {
  it('puts attention first, newest activity within groups, and unknown activity last', () => {
    const epics = [
      { ...base, id: 'routine', last_activity_at: '2026-09-06T12:00:00Z' },
      { ...base, id: 'older', awaiting_answer: 1, last_activity_at: '2026-09-05T12:00:00Z' },
      { ...base, id: 'unknown', last_activity_at: null },
      { ...base, id: 'newer', by_category: { blocked: 2 }, last_activity_at: '2026-09-06T10:00:00Z' },
    ]
    expect(applyEpicsGrid(epics, DEFAULT_FILTERS, 'attention').map((e) => e.id)).toEqual(['newer', 'older', 'routine', 'unknown'])
    expect(applyEpicsGrid(epics, DEFAULT_FILTERS, 'activity').map((e) => e.id)).toEqual(['routine', 'newer', 'older', 'unknown'])
  })
  it('does not mistake empty epics or reserved work for blocked work', () => {
    expect(needsAttention({ ...base, flags: ['empty_epic'], total: 0 })).toBe(false)
    expect(needsAttention({ ...base, backlog: 12 })).toBe(false)
    expect(needsAttention({ ...base, claim })).toBe(true)
  })
  it('respects custom terminal states and composes search with advanced filters', () => {
    const epics = [base, { ...base, id: 'closed', title: 'Billing finished', state: 'released', claim }]
    expect(applyEpicsGrid(epics, DEFAULT_FILTERS, 'title', 86400, ['released']).map((e) => e.id)).toEqual(['demo-a'])
    expect(applyEpicsGrid(epics, { search: 'BILLING', scope: 'all', state: 'released', lane: 'ini-billing', claimed: 'claimed' }, 'title', 86400, ['released']).map((e) => e.id)).toEqual(['closed'])
    expect(applyEpicsGrid(epics, { ...DEFAULT_FILTERS, search: 'demo-a' }, 'title').map((e) => e.id)).toEqual(['demo-a'])
  })
})

describe('EpicsView', () => {
  it('renders title, initiative, state and task-count progress with an actionable row', () => {
    const onOpen = view([base])
    const row = screen.getByRole('button', { name: /Billing v2/ })
    expect(row.textContent).toContain('Billing')
    expect(row.textContent).toContain('8/12 done')
    expect(row.textContent).toContain('Ready')
    expect(row.textContent).not.toContain('67%')
    fireEvent.click(row)
    expect(onOpen).toHaveBeenCalledWith('demo-a')
  })
  it('surfaces questions attached directly to an epic without tasks', () => {
    const epic = { ...base, total: 0, own_open_questions: 2 }
    expect(needsAttention(epic)).toBe(true)
    view([epic])
    expect(screen.getByText('2 open questions on this epic')).toBeTruthy()
    expect(screen.queryByText(/tasks awaiting answers/)).toBeNull()
  })
  it('keeps the heading and creation action when empty', () => {
    const onCreate = vi.fn()
    view([], { canCreate: true, onCreate })
    expect(screen.getByRole('heading', { name: 'Epics' })).toBeTruthy()
    expect(screen.getByText(labels.empty)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'New epic' }))
    expect(onCreate).toHaveBeenCalledOnce()
    expect(screen.queryByRole('list')).toBeNull()
  })
  it('does not offer creation to a read-only viewer', () => {
    view([], { canCreate: false, onCreate: vi.fn() })
    expect(screen.queryByRole('button', { name: 'New epic' })).toBeNull()
  })
  it('shows no-task progress quietly and omits zero attention labels', () => {
    view([{ ...base, total: 0, done: 0, percent: 0, flags: ['empty_epic'] }])
    const row = screen.getByRole('button', { name: /Billing v2/ })
    expect(row.textContent).toContain('No tasks')
    expect(row.textContent).not.toContain('0/0')
    expect(row.textContent).not.toContain('awaiting')
    expect(row.textContent).not.toContain('Empty epic')
  })
  it('shows factual attention labels and human-readable contradictions', () => {
    view([{ ...base, awaiting_answer: 2, by_category: { blocked: 3 }, claim, flags: ['open_with_all_children_done'] }])
    expect(screen.getByText('2 tasks awaiting answers')).toBeTruthy()
    expect(screen.getByText('3 blocked tasks')).toBeTruthy()
    expect(screen.getByText(/No movement for/)).toBeTruthy()
    expect(screen.getByText(labels.openWithDone)).toBeTruthy()
    expect(screen.queryByText('open_with_all_children_done')).toBeNull()
  })
  it('shows unclaimed activity and honest unknown on older API responses', () => {
    vi.spyOn(Date, 'now').mockReturnValue(Date.parse('2026-09-06T10:00:00Z'))
    view([base, { ...base, id: 'unknown', title: 'Unknown', last_activity_at: undefined }])
    expect(screen.getByText('Updated 1h ago')).toBeTruthy()
    expect(screen.getByText(labels.unknown)).toBeTruthy()
  })
  it('keeps advanced filters in a popover with count, summary and clear', () => {
    view([base, { ...base, id: 'other', title: 'Other', claim, initiatives: [] }])
    expect(screen.queryByLabelText('Claim')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Filters' }))
    choose('Claim', 'Unclaimed')
    expect(screen.getByRole('button', { name: 'Filters (1)' })).toBeTruthy()
    expect(screen.getByLabelText('Applied filters').textContent).toContain('Unclaimed')
    expect(screen.queryByRole('button', { name: /Other/ })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Clear' }))
    expect(screen.getByRole('button', { name: /Other/ })).toBeTruthy()
  })
  it('searches epics, toggles completed work and explains no matching results', () => {
    view([base, { ...base, id: 'done', title: 'Finished', state: 'done', state_category: 'done' }])
    expect(screen.queryByRole('button', { name: /Finished/ })).toBeNull()
    fireEvent.click(within(screen.getByRole('group', { name: labels.visibility })).getByRole('button', { name: 'All' }))
    expect(screen.getByRole('button', { name: /Finished/ })).toBeTruthy()
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'missing' } })
    expect(screen.getByText(labels.noMatch)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }))
    expect(screen.getByRole('button', { name: /Finished/ })).toBeTruthy()
  })
})
