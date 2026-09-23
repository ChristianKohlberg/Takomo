import { createContext, useContext } from 'react'
import type { Locale } from '@/lib/i18n'
import type { Project } from '@/lib/initiatives'
import type { Mindmap, MindmapSession } from '@/lib/mindmaps'
import type { PlanNode } from '@/lib/plan-sections'
import type { VerificationSummary } from '@/lib/behaviors'
import type { SyncConnection } from '@/hooks/useSyncConnection'

export interface SpecificationState {
  focusMode?: boolean
  structureHistory?: ReturnType<typeof import('@/lib/plan-structure').createStructureHistory> | null
  token: string
  lang: Locale
  project: string
  projects: Project[]
  actor: string
  userId?: string
  scopes: string[]
  voice: boolean
  map: Mindmap | null
  session: MindmapSession | null
  saveState?: import('@/lib/save-status').SaveState
  serverSync?: import('@/lib/save-status').ServerSync
  connection: SyncConnection | null
  nodes: PlanNode[]
  /** Status counts per section, from `/verification`; null until loaded. */
  verification: VerificationSummary | null
  refreshMap: () => Promise<Mindmap | null>
  refreshVerification: () => Promise<VerificationSummary | null>
  selectProject: (id: string) => void
  onError: (error: unknown) => void
  openTests: (section: string | null) => void
  /** Select a behavior (`?behavior=`), or clear the selection with null. */
  openBehavior: (id: string | null) => void
  testsFor: (section: string) => { total: number; failing: number }
}
export const SpecificationContext = createContext<SpecificationState | null>(null)
export function useSpecification() {
  const value = useContext(SpecificationContext)
  if (!value) throw new Error('A specification view needs its workspace')
  return value
}
