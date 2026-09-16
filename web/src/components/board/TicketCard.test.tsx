import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { TicketCard } from './TicketCard'
import type { Ticket } from '@/lib/board'

afterEach(cleanup)

const ticket: Ticket = {
  id: 'TK-42', project: 'demo', title: 'Fix billing retries', state: 'ready',
  priority: 'normal', labels: ['billing'], tags: ['team:payments'],
  schedule: 'schedule-123', updated_at: '2026-01-01T00:00:00Z',
}

const scheduleLabels = { fromSchedule: 'From schedule', notFulfilled: 'Missed' }

describe('TicketCard', () => {
  it('keeps routine cards focused and opens their details', () => {
    const onOpen = vi.fn()
    const { container } = render(<TicketCard ticket={ticket} onOpen={onOpen} />)
    expect(container.textContent).toBe(ticket.title)
    fireEvent.click(screen.getByRole('button', { name: ticket.title }))
    expect(onOpen).toHaveBeenCalledWith(ticket.id)
  })

  it('shows ownership and attention-worthy exceptions', () => {
    render(<TicketCard
      ticket={{ ...ticket, priority: 'critical', claim: { holder: 'agent:billing' }, blocked_by: ['TK-12'], expires_at: '2020-01-01T00:00:00Z' }}
      blockedLabel="Blocked" needsAnswerLabel="Needs an answer" scheduleLabels={scheduleLabels} onOpen={() => {}}
    />)
    for (const text of ['agent:billing', 'critical', 'Blocked', 'Needs an answer', 'Missed']) {
      expect(screen.getByText(text)).toBeTruthy()
    }
  })

  it('does not flag completed occurrences as missed', () => {
    render(<TicketCard ticket={{ ...ticket, expires_at: '2020-01-01T00:00:00Z' }}
      isDone scheduleLabels={scheduleLabels} onOpen={() => {}} />)
    expect(screen.queryByText('Missed')).toBeNull()
  })
})
