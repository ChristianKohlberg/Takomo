// The roadmap rollup: progress at two altitudes, from one request.
//
// `GET /v1/projects/{p}/roadmap` reports every epic with a rollup over its
// subtree, and every initiative with a rollup over the work tagged into it —
// because an epic CLOSES and an initiative does not. A feature that ships as v1,
// then v1.1, then v2 is one initiative with one epic per version filed under it.
//
// A lane names its versions as bare ids, not as nested objects, so the payload
// carries each epic's numbers exactly once however many lanes point at it. That
// makes joining the reader's job, which is what `laneVersions` is for.
import { api } from './api'

/** A bucket rollup: the shape shared by every entry here, minus identity. */
export interface Counts {
  total: number
  done: number
  percent: number
  ready: number
  backlog: number
  awaiting_answer: number
  by_state?: Record<string, number>
  by_category?: Record<string, number>
}

/**
 * An epic's ACTIVE claim, or null. The cheap half of `GET /v1/tickets/{id}/claim`
 * — enough to scan a list by, with that route the precise answer for one epic.
 */
export interface EpicClaim {
  holder: string
  held_since: string | null
  held_for_seconds: number | null
  /** An active claim with no expiry: held until released, so movement judges it. */
  indefinite: boolean
  expires_at: string | null
  last_activity_at: string | null
  idle_seconds: number | null
}

export interface RoadmapEpic extends Counts {
  id: string
  title: string
  state: string
  state_category: string
  priority: string
  /** Newest update on the epic or its descendants, independent of a claim. */
  own_open_questions?: number
  last_activity_at?: string | null
  /** Ids of the lanes this epic is filed under — the inverse of a lane's `epics`. */
  initiatives?: string[]
  claim?: EpicClaim | null
  flags: string[]
}

export interface RoadmapLane extends Counts {
  id: string
  title: string
  status: string
  /** Ids of the epics filed under this lane — its versions, in creation order. */
  epics: string[]
  flags: string[]
}

export interface Roadmap {
  project: string
  generated_at: string
  epics: RoadmapEpic[]
  /** Absent under an `?epic=` filter: a lane spans versions. */
  initiatives?: RoadmapLane[]
  uninitiated?: Counts
  unparented?: Counts
  /** Echoed back only when the report was narrowed to one epic. */
  epic?: string
}

export function fetchRoadmap(token: string, project: string): Promise<Roadmap> {
  return api<Roadmap>(token, `/projects/${encodeURIComponent(project)}/roadmap`)
}

/** The lane for one initiative, or undefined when it owns no work yet. */
export function lane(rm: Roadmap | undefined, initiativeId: string): RoadmapLane | undefined {
  return rm?.initiatives?.find((l) => l.id === initiativeId)
}

/**
 * The versions filed under one initiative, resolved to full epic rollups and
 * kept in the lane's own order — creation order, which for versions is the order
 * they were planned in and therefore the order to read them in.
 *
 * An id with no matching epic is DROPPED rather than rendered as a blank row.
 * That is not defensive noise: `?epic=` responses carry one epic and no lanes,
 * and a caller holding a stale full response while a version is being deleted
 * would otherwise show a row with no title.
 */
export function laneVersions(rm: Roadmap | undefined, initiativeId: string): RoadmapEpic[] {
  const l = lane(rm, initiativeId)
  if (!l || !rm) return []
  const byId = new Map(rm.epics.map((e) => [e.id, e]))
  const out: RoadmapEpic[] = []
  for (const id of l.epics) {
    const e = byId.get(id)
    if (e) out.push(e)
  }
  return out
}

/**
 * Which of a lane's flags to show, as a stable order.
 *
 * `empty_initiative` is deliberately NOT surfaced here: a lane with no work is
 * already legible as "no versions yet", and badging it as a contradiction would
 * make opening an initiative before filing work look like a mistake. It is a
 * flag in the API so a client CAN distinguish the case, not an instruction to
 * shout about it.
 */
export const LANE_FLAGS_SHOWN = ['parked_with_ready_work'] as const

export function laneWarnings(l: RoadmapLane | undefined): string[] {
  if (!l) return []
  return LANE_FLAGS_SHOWN.filter((f) => l.flags.includes(f))
}

/** Lane id → title, for naming the lanes an epic belongs to. */
export function laneTitles(rm: Roadmap | undefined): Record<string, string> {
  const out: Record<string, string> = {}
  for (const l of rm?.initiatives ?? []) out[l.id] = l.title
  return out
}

/** Seconds of no movement past which a held epic is worth looking at. */
export const STALLED_AFTER_SECONDS = 86_400

/**
 * What in this project wants a person's attention, counted once so a reader gets
 * the answer before scrolling.
 *
 * `stalled` is the number that only exists because an epic claim need not expire:
 * a held epic with nothing moving underneath is invisible in a ticket board, and
 * no lease is going to lapse and give it back.
 */
export interface EpicAttention {
  held: number
  stalled: number
  awaiting: number
  flagged: number
}

export function epicAttention(
  epics: RoadmapEpic[],
  stalledAfter: number = STALLED_AFTER_SECONDS,
): EpicAttention {
  let held = 0
  let stalled = 0
  let awaiting = 0
  let flagged = 0
  for (const e of epics) {
    if (e.claim) {
      held++
      if ((e.claim.idle_seconds ?? 0) >= stalledAfter) stalled++
    }
    if (e.awaiting_answer > 0) awaiting++
    if (e.flags.length > 0) flagged++
  }
  return { held, stalled, awaiting, flagged }
}
