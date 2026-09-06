import { api } from './api'

export interface SectionConversationView {
  turn_limit?: number
  conversation: { id: string } | null
  messages: { id: string; role: 'user' | 'assistant'; body: string; created_at: number; job_id: string }[]
  jobs: { id: string; status: 'queued' | 'running' | 'completed' | 'failed'; error: string | null; created_at: number }[]
}

const path = (map: string, node: string) =>
  `/mindmaps/${encodeURIComponent(map)}/nodes/${encodeURIComponent(node)}/conversation`

export function getSectionConversation(token: string, map: string, node: string, signal?: AbortSignal) {
  return api<SectionConversationView>(token, path(map, node), { signal })
}

export function sendSectionMessage(token: string, map: string, node: string, message: string, requestId: string, signal?: AbortSignal) {
  return api<SectionConversationView>(token, `${path(map, node)}/messages`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ message, request_id: requestId }),
    signal,
  })
}
