import { api } from './api'
import type { SectionConversationView } from './section-conversation'

export type DocumentAction = 'discuss' | 'grill' | 'draft_tests' | 'draft_questions'
export interface DocumentScope { section_ids: string[]; whole_document: boolean }
export interface DocumentContext { mode: 'automatic' | 'selected' | 'whole_document'; section_ids: string[]; pinned_section_ids: string[]; quote?: { section_id: string; text: string } }
export interface DocumentSource { section_id: string; title: string; version: string }
export interface DocumentConversationView extends Omit<SectionConversationView, 'jobs'> {
  pinned_section_ids?: string[]
  jobs: (SectionConversationView['jobs'][number] & DocumentScope & { action: DocumentAction; migration?: { previous_thread_id: string; new_thread_id: string; retained_turns: number; omitted_turns: number }; context?: DocumentContext; sources?: DocumentSource[]; source_revision?: string; coverage?: { read_section_ids: string[]; total_sections: number; complete: boolean }; section_count: number; sections?: { id: string; title: string }[] })[]
}
export interface DocumentRequest {
  context: DocumentContext
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

export function setDocumentPins(token: string, map: string, pinned_section_ids: string[], signal?: AbortSignal) {
  return api<DocumentConversationView>(token, path(map), { method: 'PATCH', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ pinned_section_ids }), signal })
}
