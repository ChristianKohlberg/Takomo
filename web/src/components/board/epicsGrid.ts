import { STALLED_AFTER_SECONDS, type RoadmapEpic } from '@/lib/roadmap'

export type ClaimedFilter = 'all' | 'claimed' | 'unclaimed'
export type EpicsSort = 'attention' | 'activity' | 'title' | 'progress'
export interface EpicsFilters {
  search: string
  scope: 'active' | 'all'
  state: string
  lane: string
  claimed: ClaimedFilter
}
export const DEFAULT_FILTERS: EpicsFilters = {
  search: '', scope: 'active', state: '', lane: '', claimed: 'all',
}
export const DEFAULT_SORT: EpicsSort = 'attention'

export function isStalled(epic: RoadmapEpic, threshold = STALLED_AFTER_SECONDS): boolean {
  return epic.claim != null && (epic.claim.idle_seconds ?? 0) >= threshold
}
export function epicWarnings(epic: RoadmapEpic): string[] {
  // Planning an epic before its tasks is ordinary, not an attention signal.
  return epic.flags.filter((flag) => flag !== 'empty_epic')
}
export function needsAttention(epic: RoadmapEpic, threshold = STALLED_AFTER_SECONDS): boolean {
  return (epic.own_open_questions ?? 0) > 0 || epic.awaiting_answer > 0 || (epic.by_category?.blocked ?? 0) > 0
    || epic.state_category === 'blocked' || isStalled(epic, threshold) || epicWarnings(epic).length > 0
}
export function activityAt(epic: RoadmapEpic): string | null {
  return epic.last_activity_at ?? epic.claim?.last_activity_at ?? null
}
function compareActivity(a: RoadmapEpic, b: RoadmapEpic): number {
  const at = Date.parse(activityAt(a) ?? '')
  const bt = Date.parse(activityAt(b) ?? '')
  // Unknown dates stay last, including on older servers that omit unclaimed activity.
  if (!Number.isFinite(at)) return Number.isFinite(bt) ? 1 : 0
  if (!Number.isFinite(bt)) return -1
  return bt - at
}
export function activeFilterCount(filters: EpicsFilters): number {
  return Number(!!filters.state) + Number(!!filters.lane) + Number(filters.claimed !== 'all')
}
export function applyEpicsGrid(
  epics: RoadmapEpic[], filters: EpicsFilters, sort: EpicsSort,
  threshold = STALLED_AFTER_SECONDS, terminalStates?: string[],
): RoadmapEpic[] {
  const search = filters.search.trim().toLocaleLowerCase()
  return epics.filter((epic) => {
    const terminal = terminalStates ? terminalStates.includes(epic.state)
      : ['done', 'cancelled'].includes(epic.state_category)
    return (filters.scope === 'all' || !terminal)
      && (!search || `${epic.title} ${epic.id}`.toLocaleLowerCase().includes(search))
      && (!filters.state || epic.state === filters.state)
      && (!filters.lane || (epic.initiatives ?? []).includes(filters.lane))
      && (filters.claimed === 'all' || (filters.claimed === 'claimed') === !!epic.claim)
  }).sort((a, b) => {
    const tie = a.title.localeCompare(b.title) || a.id.localeCompare(b.id)
    if (sort === 'title') return tie
    if (sort === 'progress') {
      if (!a.total || !b.total) return Number(!a.total) - Number(!b.total) || tie
      return b.percent - a.percent || tie
    }
    if (sort === 'attention') {
      const attention = Number(needsAttention(b, threshold)) - Number(needsAttention(a, threshold))
      if (attention) return attention
    }
    return compareActivity(a, b) || tie
  })
}
