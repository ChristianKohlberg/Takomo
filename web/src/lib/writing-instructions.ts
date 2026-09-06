import { api } from './api'

export interface WritingTemplate {
  id: string
  name: string
  instruction: string
}

export interface WritingInstructions {
  templates: WritingTemplate[]
  default_id: string | null
}

export const TEMPLATE_LIMIT = 20
export const NAME_LIMIT = 80
export const INSTRUCTION_LIMIT = 4000

const path = (project: string) => `/projects/${encodeURIComponent(project)}/writing-instructions`

export function getWritingInstructions(token: string, project: string, signal?: AbortSignal) {
  return api<WritingInstructions>(token, path(project), { signal })
}

export function saveWritingInstructions(token: string, project: string, value: WritingInstructions) {
  return api<WritingInstructions>(token, path(project), {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(value),
  })
}
