import { api } from './api'
export interface DefinitionSnapshot {
  id: string; title: string; body: string; precondition: string | null; node: string | null
  layer: string; severity: string; verification: string; environments: string[]
  cases: { id: string; key: string; label: string; assignment: unknown }[]
}
export interface TestDefinition {
  id: string; definition_revision: string; specification_revision: string | null
  definition: DefinitionSnapshot
  execution: { state: string; environments: { run?: string; environment?: string; code_ref?: string; state: string }[] }
}
export interface RunResult { id: string; actor_kind: 'agent' | 'human'; actor: string; verdict: string; note: string | null; evidence: string[]; recorded_at: string }
export interface RunCase {
  case: string; check: string; definition_revision: string | null; specification_revision: string | null
  revision_known: boolean
  snapshot: DefinitionSnapshot['cases'][number] | null; results: RunResult[]
}
export interface TestRun {
  id: string; project: string; kind: 'execution' | 'legacy'; status: string; code_ref: string | null
  environment: string | null; environment_snapshot: { name: string; slug: string } | null
  created_by: string; executor: string | null; created_at: string; started_at: string | null; finished_at: string | null
  retry_of: string | null; cases: RunCase[]
  definitions: Record<string, {
    definition_revision: string | null; specification_revision: string | null
    definition: Omit<DefinitionSnapshot, 'cases'> | null
    specification: { sections: { id: string; title: string; notes: string | null }[] } | null
  }>
}
export interface RunSummary extends Omit<TestRun, 'cases' | 'definitions'> { case_count: number; checks: string[] }
export interface RunPage { items: RunSummary[]; next_cursor: string | null; total: number }
export const runRequest = <T>(token: string, path: string, body: unknown, method = 'POST') => api<T>(token, path, { method, headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) })
export async function listDefinitions(token: string, project: string) {
  const items: TestDefinition[] = []
  let offset: number | null = 0
  do {
    const page: { items: TestDefinition[]; next_offset: number | null } = await api(token, `/projects/${encodeURIComponent(project)}/test-definitions?limit=100&offset=${offset}`)
    items.push(...page.items)
    offset = page.next_offset
  } while (offset !== null)
  return items
}
