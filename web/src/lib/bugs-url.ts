// The /bugs view, as a URL: the project, the selected bug and every filter, so
// a reload, Back/Forward and a shared link all land on the same queue. Defaults
// are omitted so an unfiltered queue has a clean URL. `readBugsView` and
// `writeBugsView` are each other's inverse.
export const BUG_VIEWS = ['open', 'needs_triage', 'ready_for_review', 'in_progress', 'all'] as const
export const BUG_SEVERITIES = ['unknown', 'critical', 'high', 'medium', 'low'] as const
export const RESEARCH_STATUSES = ['none', 'queued', 'running', 'completed', 'failed', 'cancelled'] as const
export const PAGE_SIZE = 50
export interface BugsView {
  project: string
  view: string
  severity: string
  search: string
  state: string
  assignee: string
  researchStatus: string
  offset: number
  bug: string
}
export const DEFAULT_BUGS_VIEW: BugsView = { project: '', view: 'open', severity: '', search: '', state: '', assignee: '', researchStatus: '', offset: 0, bug: '' }
function oneOf(value: string | null, allowed: readonly string[]): string {
  return value && allowed.includes(value) ? value : ''
}
export function readBugsView(search: string | URLSearchParams): BugsView {
  const p = search instanceof URLSearchParams ? search : new URLSearchParams(search)
  const offset = Number.parseInt(p.get('offset') ?? '', 10)
  return {
    project: p.get('project') ?? '',
    view: oneOf(p.get('view'), BUG_VIEWS) || 'open',
    severity: oneOf(p.get('severity'), BUG_SEVERITIES),
    search: p.get('q') ?? '',
    state: p.get('state') ?? '',
    assignee: p.get('assignee') ?? '',
    researchStatus: oneOf(p.get('research'), RESEARCH_STATUSES),
    offset: Number.isFinite(offset) && offset > 0 ? offset - (offset % PAGE_SIZE) : 0,
    bug: p.get('bug') ?? '',
  }
}
export function writeBugsView(v: BugsView): URLSearchParams {
  const p = new URLSearchParams()
  if (v.project) p.set('project', v.project)
  if (v.view !== 'open') p.set('view', v.view)
  if (v.severity) p.set('severity', v.severity)
  if (v.search) p.set('q', v.search)
  if (v.state) p.set('state', v.state)
  if (v.assignee) p.set('assignee', v.assignee)
  if (v.researchStatus) p.set('research', v.researchStatus)
  if (v.offset > 0) p.set('offset', String(v.offset))
  if (v.bug) p.set('bug', v.bug)
  return p
}
/** A link that opens one project's queue, optionally on one bug, without depending on stored state. */
export function bugsLink(project: string, bug?: string): string {
  const s = writeBugsView({ ...DEFAULT_BUGS_VIEW, project, bug: bug ?? '' }).toString()
  return s ? `/bugs?${s}` : '/bugs'
}
