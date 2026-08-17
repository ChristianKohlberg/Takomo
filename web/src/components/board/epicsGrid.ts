// Filter, sort, and preset logic for the epics grid — kept out of the component
// so preset definitions stay testable without a DOM.
import type { RoadmapEpic } from '@/lib/roadmap'

export type SortKey = 'title' | 'state' | 'progress' | 'holder' | 'lastActivity' | 'creation'
export type SortDir = 'asc' | 'desc'
export type ClaimedFilter = 'all' | 'claimed' | 'unclaimed'

/** One-click views onto common questions. Each preset's filter is in `matchesPreset`. */
export type PresetId =
  | 'recentCreated'
  | 'nearlyComplete'
  | 'notStarted'
  | 'stalled'
  | 'awaiting'
  | 'unclaimed'
  | 'flagged'

export const PRESET_IDS: PresetId[] = [
  'recentCreated',
  'nearlyComplete',
  'notStarted',
  'stalled',
  'awaiting',
  'unclaimed',
  'flagged',
]

export interface EpicsFilters {
  /** Empty string means every category. */
  stateCategory: string
  /** Empty string means every lane. */
  lane: string
  claimed: ClaimedFilter
}

export interface EpicsSort {
  key: SortKey
  dir: SortDir
}

export const DEFAULT_FILTERS: EpicsFilters = {
  stateCategory: '',
  lane: '',
  claimed: 'all',
}

export const DEFAULT_SORT: EpicsSort = { key: 'creation', dir: 'asc' }

/** Server order is creation order — the index is the only honest "recently created" signal. */
export function creationIndex(epics: RoadmapEpic[], id: string): number {
  return epics.findIndex((e) => e.id === id)
}

export function matchesFilters(e: RoadmapEpic, filters: EpicsFilters): boolean {
  if (filters.stateCategory && e.state_category !== filters.stateCategory) return false
  if (filters.lane && !(e.initiatives ?? []).includes(filters.lane)) return false
  if (filters.claimed === 'claimed' && !e.claim) return false
  if (filters.claimed === 'unclaimed' && e.claim) return false
  return true
}

/**
 * Preset filters — documented here so the PR and tests share one definition.
 *
 * - recentCreated: no filter (sort only — newest creation index first)
 * - nearlyComplete: has work, 75–99% done
 * - notStarted: zero children completed (`done === 0`)
 * - stalled: active claim idle ≥ threshold (same predicate as `epicAttention`)
 * - awaiting: `awaiting_answer > 0`
 * - unclaimed: no active claim
 * - flagged: at least one contradiction flag
 */
export function matchesPreset(e: RoadmapEpic, preset: PresetId, stalledAfter: number): boolean {
  switch (preset) {
    case 'recentCreated':
      return true
    case 'nearlyComplete':
      return e.total > 0 && e.percent >= 75 && e.percent < 100
    case 'notStarted':
      return e.done === 0
    case 'stalled':
      return e.claim != null && (e.claim.idle_seconds ?? 0) >= stalledAfter
    case 'awaiting':
      return e.awaiting_answer > 0
    case 'unclaimed':
      return !e.claim
    case 'flagged':
      return e.flags.length > 0
  }
}

/** Sort each preset applies when active — recentCreated is the only one that reorders by creation. */
export function presetSort(preset: PresetId): EpicsSort {
  switch (preset) {
    case 'recentCreated':
      return { key: 'creation', dir: 'desc' }
    case 'nearlyComplete':
      return { key: 'progress', dir: 'desc' }
    case 'notStarted':
      return { key: 'progress', dir: 'asc' }
    default:
      return { key: 'title', dir: 'asc' }
  }
}

function compareNullableString(a: string | null | undefined, b: string | null | undefined): number {
  if (!a && !b) return 0
  if (!a) return 1
  if (!b) return -1
  return a.localeCompare(b)
}

export function compareEpics(
  a: RoadmapEpic,
  b: RoadmapEpic,
  sort: EpicsSort,
  epics: RoadmapEpic[],
): number {
  const mul = sort.dir === 'asc' ? 1 : -1
  let cmp = 0
  switch (sort.key) {
    case 'creation':
      cmp = creationIndex(epics, a.id) - creationIndex(epics, b.id)
      break
    case 'title':
      cmp = a.title.localeCompare(b.title) || a.id.localeCompare(b.id)
      break
    case 'state':
      cmp = a.state.localeCompare(b.state) || a.title.localeCompare(b.title)
      break
    case 'progress':
      cmp = a.percent - b.percent || a.done - b.done || a.total - b.total
      break
    case 'holder':
      cmp = compareNullableString(a.claim?.holder, b.claim?.holder)
      break
    case 'lastActivity': {
      const at = a.claim?.last_activity_at
      const bt = b.claim?.last_activity_at
      if (!at && !bt) cmp = 0
      else if (!at) cmp = 1
      else if (!bt) cmp = -1
      else cmp = new Date(at).getTime() - new Date(bt).getTime()
      break
    }
  }
  return cmp * mul
}

export function activeFilterCount(filters: EpicsFilters): number {
  let n = 0
  if (filters.stateCategory) n++
  if (filters.lane) n++
  if (filters.claimed !== 'all') n++
  return n
}

export function applyEpicsGrid(
  epics: RoadmapEpic[],
  filters: EpicsFilters,
  sort: EpicsSort,
  preset: PresetId | null,
  stalledAfter: number,
): RoadmapEpic[] {
  let result = epics.filter((e) => matchesFilters(e, filters))
  const effectiveSort = preset ? presetSort(preset) : sort
  if (preset) {
    result = result.filter((e) => matchesPreset(e, preset, stalledAfter))
  }
  return [...result].sort((a, b) => compareEpics(a, b, effectiveSort, epics))
}
