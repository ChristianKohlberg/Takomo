import { api } from './api'
export interface SavedSection {
  id: string
  parent: string | null
  position: number
  title: string
  notes: string | null
  prose_xml?: string
  prose_structure?: unknown
  edge_label: string
  kind: string
  color: string
  shape: string
  icons: string[]
  attachments: unknown[]
  at: unknown
  reviewed?: boolean
  origin?: string
  promoted?: unknown
}
export interface SavedVersion {
  version: number
  kind: 'baseline' | 'save'
  recorded_at: string
  recorded_by: string | null
  checkpoints: {
    name: string
    actor: string
    user: string | null
    created_at: string
  }[]
}
export interface VersionDetail extends SavedVersion {
  nodes: SavedSection[]
  relationships: unknown[]
}
export interface VersionPage {
  items: SavedVersion[]
  head: number
  next_cursor: number | null
  total: number
}
const path = (map: string) => `/mindmaps/${encodeURIComponent(map)}`
export const historyPage = (
  token: string,
  map: string,
  before: number | null,
  checkpoints: boolean,
) =>
  api<VersionPage>(
    token,
    `${path(map)}/versions?limit=30&checkpoints=${checkpoints}${before === null ? '' : `&before=${before}`}`,
  )
export const savedVersion = (token: string, map: string, version: number) =>
  api<VersionDetail>(token, `${path(map)}/versions/${version}`)
export const checkpoint = (
  token: string,
  map: string,
  version: number,
  name: string,
) =>
  api<SavedVersion>(token, `${path(map)}/checkpoints`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ expected_version: version, name }),
  })
const fields = [
  'title',
  'notes',
  'prose_xml',
  'parent',
  'position',
  'edge_label',
  'kind',
  'color',
  'shape',
  'icons',
  'attachments',
  'at',
  'reviewed',
  'origin',
  'promoted',
] as const
export function compareVersions(before: VersionDetail, after: VersionDetail) {
  const old = new Map(before.nodes.map((node) => [node.id, node]))
  const current = new Map(after.nodes.map((node) => [node.id, node]))
  return [...new Set([...old.keys(), ...current.keys()])].flatMap((id) => {
    const a = old.get(id),
      b = current.get(id)
    const changed =
      a && b
        ? fields.filter(
            (field) =>
              JSON.stringify(
                field === 'prose_xml'
                  ? (a.prose_structure ?? a[field])
                  : a[field],
              ) !==
              JSON.stringify(
                field === 'prose_xml'
                  ? (b.prose_structure ?? b[field])
                  : b[field],
              ),
          )
        : []
    return !a || !b || changed.length
      ? [
          {
            id,
            before: a,
            after: b,
            changed,
            kind: !a ? 'added' : !b ? 'removed' : 'changed',
          },
        ]
      : []
  })
}

/** Preserve loaded older pages only while the refreshed page overlaps them.
 * Otherwise reset the cursor: retaining them would hide the intervening gap. */
export function mergeHistoryPage(
  current: VersionPage | null,
  next: VersionPage,
): VersionPage {
  if (
    !current ||
    !next.items.some((item) =>
      current.items.some((old) => old.version === item.version),
    )
  )
    return next
  const older = current.items.filter(
    (item) => item.version < (next.items.at(-1)?.version ?? Infinity),
  )
  return older.length
    ? {
        ...next,
        items: [...next.items, ...older],
        next_cursor: current.next_cursor,
      }
    : next
}
