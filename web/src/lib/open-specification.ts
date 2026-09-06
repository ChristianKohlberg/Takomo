import type { ApiErrorShape } from './api'
import { createMindmap, listMindmaps, type Mindmap } from './mindmaps'

/** Opening a project's specification needs no separate creation workflow.
 * The server enforces one map per project, including simultaneous first visits.
 * Readers can open an existing plan without being asked to perform a write. */
export async function openSpecification(
  token: string,
  project: string,
  title: string,
  canWrite: boolean,
): Promise<Mindmap | null> {
  const read = async () => (await listMindmaps(token, { project, limit: 1 })).items[0] ?? null
  const existing = await read()
  if (existing || !canWrite) return existing
  try {
    return (await createMindmap(token, { project, title: Array.from(title || project).slice(0, 300).join('') })).mindmap
  } catch (error) {
    // Another tab may have created it between our read and write. Network
    // retries also arrive here when a successful response was lost.
    if ((error as ApiErrorShape).code !== 'mindmap.project_has_one') throw error
    const created = await read()
    if (!created) throw error
    return created
  }
}
