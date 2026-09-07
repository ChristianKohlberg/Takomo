import { api } from './api'
import type { SectionConversationView } from './section-conversation'

export type DocumentAction = 'discuss' | 'grill' | 'draft_tests' | 'draft_questions'
export interface DocumentScope { section_ids: string[]; whole_document: boolean }
export interface DocumentConversationView extends Omit<SectionConversationView, 'jobs'> {
  jobs: (SectionConversationView['jobs'][number] & DocumentScope & { action: DocumentAction; section_count: number; sections?: { id: string; title: string }[] })[]
}
export interface DocumentRequest extends DocumentScope {
  message: string
  request_id: string
  action: DocumentAction
}
const path = (map: string) => `/mindmaps/${encodeURIComponent(map)}/conversation`
export function getDocumentConversation(token: string, map: string, signal?: AbortSignal) {
  return api<DocumentConversationView>(token, path(map), { signal })
}
export function sendDocumentMessage(token: string, map: string, request: DocumentRequest, signal?: AbortSignal) {
  return api<DocumentConversationView>(token, `${path(map)}/messages`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(request), signal,
  })
}
