import { useState } from 'react'
import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { OutlineRail } from './OutlineRail'
import { planSections } from '@/lib/plan-sections'
const sections = planSections([
  { id: 'a', parent: null, title: 'Payments', position: 0, order: 'a' },
  { id: 'b', parent: 'a', title: 'Retries', position: 0, order: 'a' },
  { id: 'c', parent: 'b', title: 'Timeouts', position: 0, order: 'a' },
  { id: 'd', parent: null, title: 'Receipts', position: 1, order: 'b' },
])
const labels = { outline: 'Outline', expand: 'Expand section', collapse: 'Collapse section', folded: '{n} hidden', untitled: 'Untitled', standingConfirmed: 'Confirmed', standingChanged: 'Changed', standingUnseen: 'Unread', pending: '{n} proposals' }
const common = { sections, selected: 'a', labels }
function Controlled({ onSelect = vi.fn() }: { onSelect?: (id: string) => void }) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set(['a']))
  return <OutlineRail {...common} onSelect={onSelect} collapsed={collapsed} onToggle={id => setCollapsed(current => { const next = new Set(current); if (next.has(id)) next.delete(id); else next.add(id); return next })} />
}
const row = (name: string) => screen.getByRole('treeitem', { name })
const transfer = () => ({ setData: vi.fn(), effectAllowed: '', dropEffect: '' })
function drag(node: HTMLElement, type: string, dataTransfer: ReturnType<typeof transfer>, clientY = 120) {
  const event = new Event(type, { bubbles: true, cancelable: true })
  Object.assign(event, { dataTransfer, clientY }); fireEvent(node, event)
}
function pointer(node: HTMLElement, type: string, x = 0, y = 0) {
  const event = new Event(type, { bubbles: true, cancelable: true })
  Object.assign(event, { pointerType: 'touch', clientX: x, clientY: y }); fireEvent(node, event)
}
afterEach(() => vi.useRealTimers())
describe('outline tree interaction', () => {
  it('hides only the configured top-level number prefixes', () => {
    const view = render(<OutlineRail {...common} onSelect={vi.fn()} collapsed={new Set()} onToggle={vi.fn()} numbering={{ h1: false, h2: false }} />)
    expect(row('Payments')).toBeTruthy()
    expect(row('Retries')).toBeTruthy()
    expect(row('1.1.1 Timeouts')).toBeTruthy()
    view.rerender(<OutlineRail {...common} onSelect={vi.fn()} collapsed={new Set()} onToggle={vi.fn()} numbering={{ h1: true, h2: false }} />)
    expect(row('1 Payments')).toBeTruthy()
    expect(row('Retries')).toBeTruthy()
  })
  it('uses standard tree navigation without jumping the document until explicitly selected', () => {
    const select = vi.fn(); render(<Controlled onSelect={select} />)
    const payments = row('1 Payments'); payments.focus()
    fireEvent.keyDown(payments, { key: 'ArrowRight' })
    expect(payments.getAttribute('aria-expanded')).toBe('true')
    fireEvent.keyDown(payments, { key: 'ArrowRight' })
    expect(document.activeElement).toBe(row('1.1 Retries'))
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' })
    expect(document.activeElement).toBe(row('1.1.1 Timeouts'))
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowLeft' })
    expect(document.activeElement).toBe(row('1.1 Retries'))
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowLeft' })
    expect(screen.queryByRole('treeitem', { name: '1.1.1 Timeouts' })).toBeNull()
    fireEvent.keyDown(document.activeElement!, { key: 'End' })
    expect(document.activeElement).toBe(row('2 Receipts'))
    fireEvent.keyDown(document.activeElement!, { key: 'Home' })
    expect(document.activeElement).toBe(payments)
    expect(select).not.toHaveBeenCalled()
    fireEvent.keyDown(payments, { key: 'Enter' })
    expect(select).toHaveBeenLastCalledWith('a')
    expect(row('1.1 Retries').getAttribute('aria-level')).toBe('2')
    expect(row('2 Receipts').getAttribute('aria-posinset')).toBe('2')
    expect(row('2 Receipts').getAttribute('aria-setsize')).toBe('2')
  })
  it('provides named icon-only expand/collapse-all controls', () => {
    render(<Controlled />)
    const expand = screen.getByRole('button', { name: 'Expand all sections' })
    expect(expand.textContent).toBe('')
    fireEvent.click(expand)
    expect(row('1.1.1 Timeouts')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Collapse all sections' }))
    expect(screen.getAllByRole('treeitem')).toHaveLength(2)
  })
  it.each([[101, 'before'], [120, 'child'], [139, 'after']] as const)('reorders whole rows through the %s drop zone as %s', (y, placement) => {
    const reorder = vi.fn().mockReturnValue({ ok: true })
    render(<OutlineRail {...common} onSelect={vi.fn()} collapsed={new Set()} onToggle={vi.fn()} onReorder={reorder} />)
    const target = row('1 Payments'); vi.spyOn(target, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 100, 200, 40))
    const data = transfer(); drag(row('2 Receipts'), 'dragstart', data); drag(target, 'dragover', data, y); drag(target, 'drop', data, y)
    expect(reorder).toHaveBeenCalledWith('d', 'a', placement)
    expect(screen.getByRole('status').textContent).toContain('Moved:')
  })
  it('rejects self/descendant and external drops, and shows concurrent move failures', () => {
    const reorder = vi.fn().mockReturnValue({ ok: false, error: 'changed' })
    render(<OutlineRail {...common} onSelect={vi.fn()} collapsed={new Set()} onToggle={vi.fn()} onReorder={reorder} />)
    const data = transfer(); drag(row('1 Payments'), 'dragstart', data); drag(row('1.1 Retries'), 'drop', data)
    expect(reorder).not.toHaveBeenCalled()
    drag(row('2 Receipts'), 'drop', data)
    expect(reorder).not.toHaveBeenCalled()
    drag(row('2 Receipts'), 'dragstart', data); drag(row('1 Payments'), 'drop', data)
    expect(screen.getByRole('status').textContent).toContain('not moved')
  })
  it('offers the safe destination picker on touch hold and cancels it when scrolling', () => {
    vi.useFakeTimers(); const move = vi.fn(); const select = vi.fn()
    render(<OutlineRail {...common} onSelect={select} collapsed={new Set()} onToggle={vi.fn()} onMove={move} />)
    const target = row('1 Payments')
    pointer(target, 'pointerdown'); pointer(target, 'pointermove', 0, 20)
    act(() => vi.advanceTimersByTime(600)); expect(move).not.toHaveBeenCalled()
    pointer(target, 'pointerdown'); act(() => vi.advanceTimersByTime(500))
    expect(move).toHaveBeenCalledWith('a')
    pointer(target, 'pointerup'); fireEvent.click(screen.getByRole('button', { name: '1 Payments' }))
    expect(select).not.toHaveBeenCalled()
  })
  it('does not reorder for read-only viewers or when the dragged section disappears', () => {
    const props = { ...common, onSelect: vi.fn(), collapsed: new Set<string>(), onToggle: vi.fn() }
    const ui = render(<OutlineRail {...props} />)
    expect(row('1 Payments').draggable).toBe(false)
    const reorder = vi.fn().mockReturnValue({ ok: true })
    ui.rerender(<OutlineRail {...props} onReorder={reorder} />)
    const data = transfer(); drag(row('2 Receipts'), 'dragstart', data)
    ui.rerender(<OutlineRail {...props} sections={[sections[0]!]} onReorder={reorder} />)
    drag(row('1 Payments'), 'drop', data)
    expect(reorder).not.toHaveBeenCalled()
  })
})
