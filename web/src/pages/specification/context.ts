import { createContext, useContext, type Dispatch, type SetStateAction } from 'react'
import type { Locale } from '@/lib/i18n'
import type { Project } from '@/lib/initiatives'
import type { Mindmap, MindmapSession } from '@/lib/mindmaps'
import type { Check } from '@/lib/verification'
import type { SyncConnection } from '@/hooks/useSyncConnection'

export interface SpecificationState {
  token: string
  lang: Locale
  project: string
  projects: Project[]
  actor: string
  scopes: string[]
  voice: boolean
  map: Mindmap | null
  session: MindmapSession | null
  connection: SyncConnection | null
  checks: Check[]
  setChecks: Dispatch<SetStateAction<Check[]>>
  refreshMap: () => Promise<Mindmap | null>
  refreshChecks: () => Promise<Check[]>
  selectProject: (id: string) => void
  onError: (error: unknown) => void
  openTests: (section: string | null) => void
  editCheck: (id: string) => void
  testsFor: (section: string) => { total: number; failing: number }
}
export const SpecificationContext = createContext<SpecificationState | null>(null)
export function useSpecification() {
  const value = useContext(SpecificationContext)
  if (!value) throw new Error('A specification view needs its workspace')
  return value
}
