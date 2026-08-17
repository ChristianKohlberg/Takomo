// The /inbox view, as a URL.
//
// Filter state used to live only in React state, so "everything still open on
// demo-6jje" was something you could look at and not something you could send
// anyone — and a reload dropped it. The reading pane was already deep-linkable
// (`#q=<id>`); the filters now sit in the query string beside it.
//
// Round-tripping is the contract these functions exist to keep: `readView` and
// `writeView` are each other's inverse, which is what the tests assert.
import { FOLDERS, type Folder } from './questions'
import { URGENCIES, type QuestionQuery } from './question-filters'

export interface InboxView extends QuestionQuery {
  folder: Folder
  /** Group the list by epic, with collapsible headings. */
  group: boolean
}

export const DEFAULT_VIEW: InboxView = { folder: 'open', group: false }

function bool(v: string | null): boolean {
  return v === '1' || v === 'true'
}

export function readView(search: string): InboxView {
  const p = new URLSearchParams(search)
  const folder = p.get('folder')
  const mode = p.get('mode')
  const urgency = (p.get('urgency') ?? '')
    .split(',')
    .map((s) => s.trim())
    .filter((s) => (URGENCIES as readonly string[]).includes(s))

  return {
    // An unknown folder falls back rather than rendering an empty rail
    // selection: a stale link should open the inbox, not a dead view.
    folder: FOLDERS.includes(folder as Folder) ? (folder as Folder) : 'open',
    group: bool(p.get('group')),
    ticket: p.get('ticket') ?? undefined,
    search: p.get('search') ?? undefined,
    urgency: urgency.length ? urgency : undefined,
    mode: mode === 'blocking' || mode === 'advisory' ? mode : undefined,
    mine: bool(p.get('mine')) || undefined,
    assignee: p.get('assignee') ?? undefined,
    hideAwaitingAgent: bool(p.get('waiting')) || undefined,
    expiringSoon: bool(p.get('soon')) || undefined,
    askedBy: p.get('asked_by') ?? undefined,
  }
}

/**
 * The query string for a view — defaults omitted, so an unfiltered inbox has a
 * clean URL and a shared link carries only what was actually chosen.
 */
export function writeView(v: InboxView): string {
  const p = new URLSearchParams()
  if (v.folder && v.folder !== 'open') p.set('folder', v.folder)
  if (v.group) p.set('group', '1')
  if (v.ticket) p.set('ticket', v.ticket)
  if (v.search?.trim()) p.set('search', v.search)
  if (v.urgency?.length) p.set('urgency', v.urgency.join(','))
  if (v.mode) p.set('mode', v.mode)
  if (v.mine) p.set('mine', '1')
  if (v.assignee) p.set('assignee', v.assignee)
  if (v.hideAwaitingAgent) p.set('waiting', '1')
  if (v.expiringSoon) p.set('soon', '1')
  if (v.askedBy) p.set('asked_by', v.askedBy)
  const s = p.toString()
  return s ? '?' + s : ''
}

/** Every filter dropped, the folder and the grouping kept — "clear filters". */
export function clearedFilters(v: InboxView): InboxView {
  return { folder: v.folder, group: v.group }
}
