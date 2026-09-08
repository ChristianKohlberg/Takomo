import { beforeEach, describe, expect, it, vi } from 'vitest'
import { api } from './api'
import { addTicketDocumentLink, changeTicketDocumentLink, classifyTicketDocument, classifyProjectDocuments, getProjectDocumentLinks, saveClassificationPolicy } from './ticket-document-links'
vi.mock('./api', () => ({ api: vi.fn().mockResolvedValue({}) }))
beforeEach(() => vi.clearAllMocks())
describe('document reference API requests', () => {
  it('sends only manual reference fields and explicit suggestion decisions', async () => {
    await addTicketDocumentLink('token', 'T/1', 'section', false)
    expect(api).toHaveBeenLastCalledWith('token', '/tickets/T%2F1/document-links', expect.objectContaining({ method: 'POST', body: JSON.stringify({ section_id: 'section', primary: false }) }))
    await changeTicketDocumentLink('token', 'T/1', 'L/1', { state: 'accepted', primary: true })
    expect(api).toHaveBeenLastCalledWith('token', '/tickets/T%2F1/document-links/L%2F1', expect.objectContaining({ method: 'PATCH', body: JSON.stringify({ state: 'accepted', primary: true }) }))
  })
  it('carries retry identities, policy and section pagination without caller-controlled provenance', async () => {
    await classifyTicketDocument('token', 'T1', 'retry-id')
    expect(api).toHaveBeenLastCalledWith('token', '/tickets/T1/document-classification', expect.objectContaining({ body: '{"request_id":"retry-id"}' }))
    await classifyProjectDocuments('token', 'project', 'batch-id')
    expect(api).toHaveBeenLastCalledWith('token', '/projects/project/document-classification', expect.objectContaining({ body: '{"request_id":"batch-id"}' }))
    await saveClassificationPolicy('token', 'project', 'auto_apply_clear')
    expect(api).toHaveBeenLastCalledWith('token', '/projects/project/document-classification-config', expect.objectContaining({ method: 'PUT', body: '{"mode":"auto_apply_clear"}' }))
    await getProjectDocumentLinks('token', 'project', undefined, 's/1', 500)
    expect(api).toHaveBeenLastCalledWith('token', '/projects/project/document-links?limit=500&offset=500&section_id=s%2F1', { signal: undefined })
  })
})
