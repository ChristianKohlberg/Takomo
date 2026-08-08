// Cadence and slot presentation. Pure functions, ported from schedules.html
// where they were inline and untestable.
import type { Cadence, Outcome, Unit } from './schedules'
import type { Locale } from './i18n'

const intl = (lang: Locale) => (lang === 'de' ? 'de-DE' : 'en-GB')

function pad(n: number): string {
  return n < 10 ? '0' + n : String(n)
}

/**
 * An instant the way the rest of takomo renders one: local, short, and never
 * pretending to a precision the sweeper does not have.
 */
export function fmtWhen(iso: string | null | undefined, lang: Locale): string {
  if (!iso) return '—'
  const d = new Date(iso)
  if (isNaN(d.getTime())) return iso
  return (
    d.toLocaleDateString(intl(lang), { day: 'numeric', month: 'short' }) +
    ', ' +
    pad(d.getHours()) +
    ':' +
    pad(d.getMinutes())
  )
}

/**
 * ISO-8601 week label, e.g. `2026-W32`. Computed rather than taken from the API
 * because the API sends the instant; the label is a local presentation of it.
 */
export function isoWeekLabel(d: Date): string {
  const x = new Date(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()))
  const day = x.getUTCDay() || 7
  x.setUTCDate(x.getUTCDate() + 4 - day)
  const yearStart = new Date(Date.UTC(x.getUTCFullYear(), 0, 1))
  const week = Math.ceil(((x.getTime() - yearStart.getTime()) / 86400000 + 1) / 7)
  return x.getUTCFullYear() + '-W' + pad(week)
}

/**
 * The slot label on a strip cell. Deliberately short — the cell is ~110px — and
 * derived from the cadence, so a weekly row reads as a week and a daily row as a
 * date.
 */
export function slotLabel(iso: string, unit: Unit | undefined, lang: Locale): string {
  const d = new Date(iso)
  if (isNaN(d.getTime())) return iso
  if (unit === 'week') return isoWeekLabel(d)
  if (unit === 'month') return d.toLocaleDateString(intl(lang), { month: 'short', year: '2-digit' })
  return d.toLocaleDateString(intl(lang), { day: 'numeric', month: 'short' })
}

export interface CadenceWords {
  every: string
  onDay: string
  day: string
  week: string
  month: string
  days: string
  weeks: string
  months: string
}

/** The cadence as one human line, in the reader's language. */
export function cadenceText(c: Cadence | undefined, w: CadenceWords): string {
  if (!c) return '—'
  const out: string[] = []
  if (c.interval && c.interval > 1) {
    const plural = c.every === 'day' ? w.days : c.every === 'week' ? w.weeks : w.months
    out.push(`${w.every} ${c.interval} ${plural}`)
  } else {
    out.push(c.every === 'day' ? w.day : c.every === 'week' ? w.week : w.month)
  }
  if (c.on?.length) out.push(c.on.join(' '))
  if (c.day) out.push(`${w.onDay} ${c.day}`)
  if (c.at) out.push(c.at)
  out.push(c.tz && c.tz !== 'UTC' ? c.tz : 'UTC')
  return out.join(' · ')
}

/** `open` is the fallback: an occurrence with no recorded outcome is still live. */
export function outcomeOf(o: Outcome | null | undefined): Outcome {
  return o === 'done' || o === 'not_fulfilled' ? o : 'open'
}
