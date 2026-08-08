import { describe, it, expect } from 'vitest'
import { cadenceText, fmtWhen, isoWeekLabel, outcomeOf, slotLabel, type CadenceWords } from './cadence'

const EN: CadenceWords = {
  every: 'every',
  onDay: 'on day',
  day: 'daily',
  week: 'weekly',
  month: 'monthly',
  days: 'days',
  weeks: 'weeks',
  months: 'months',
}

describe('cadenceText', () => {
  it('names the plain units', () => {
    expect(cadenceText({ every: 'day', at: '09:00' }, EN)).toBe('daily · 09:00 · UTC')
    expect(cadenceText({ every: 'week', at: '09:00' }, EN)).toBe('weekly · 09:00 · UTC')
    expect(cadenceText({ every: 'month', at: '09:00' }, EN)).toBe('monthly · 09:00 · UTC')
  })

  it('switches to "every N <plural>" only when the interval is > 1', () => {
    expect(cadenceText({ every: 'week', interval: 1, at: '09:00' }, EN)).toBe('weekly · 09:00 · UTC')
    expect(cadenceText({ every: 'week', interval: 2, at: '09:00' }, EN)).toBe(
      'every 2 weeks · 09:00 · UTC',
    )
  })

  it('includes weekdays and day-of-month', () => {
    expect(cadenceText({ every: 'week', on: ['mon', 'thu'], at: '09:00' }, EN)).toBe(
      'weekly · mon thu · 09:00 · UTC',
    )
    expect(cadenceText({ every: 'month', day: 1, at: '09:00' }, EN)).toBe(
      'monthly · on day 1 · 09:00 · UTC',
    )
  })

  it('shows a real zone but collapses an absent one to UTC', () => {
    expect(cadenceText({ every: 'day', at: '07:00', tz: 'Europe/Berlin' }, EN)).toContain(
      'Europe/Berlin',
    )
    expect(cadenceText({ every: 'day', at: '07:00', tz: 'UTC' }, EN)).toContain('UTC')
    expect(cadenceText(undefined, EN)).toBe('—')
  })
})

describe('isoWeekLabel', () => {
  it('computes the ISO week, including the year-boundary cases', () => {
    // 2026-01-01 is a Thursday, so it belongs to week 1 of 2026.
    expect(isoWeekLabel(new Date(2026, 0, 1))).toBe('2026-W01')
    expect(isoWeekLabel(new Date(2026, 7, 3))).toBe('2026-W32')
  })
})

describe('slotLabel', () => {
  const iso = new Date(2026, 7, 3, 9, 0).toISOString()

  it('derives the label from the cadence unit', () => {
    expect(slotLabel(iso, 'week', 'en')).toBe('2026-W32')
    expect(slotLabel(iso, 'month', 'en')).toMatch(/Aug/)
    expect(slotLabel(iso, 'day', 'en')).toMatch(/3/)
  })

  it('returns the input unchanged when it is not a date', () => {
    expect(slotLabel('not-a-date', 'week', 'en')).toBe('not-a-date')
  })
})

describe('fmtWhen', () => {
  it('renders a short local instant', () => {
    expect(fmtWhen(new Date(2026, 7, 3, 9, 5).toISOString(), 'en')).toMatch(/Aug, 09:05/)
  })

  it('falls back rather than inventing a time', () => {
    expect(fmtWhen(null, 'en')).toBe('—')
    expect(fmtWhen('', 'en')).toBe('—')
    expect(fmtWhen('garbage', 'en')).toBe('garbage')
  })
})

describe('outcomeOf', () => {
  it('treats an unrecorded outcome as still open', () => {
    expect(outcomeOf('done')).toBe('done')
    expect(outcomeOf('not_fulfilled')).toBe('not_fulfilled')
    expect(outcomeOf(null)).toBe('open')
    expect(outcomeOf(undefined)).toBe('open')
  })
})
