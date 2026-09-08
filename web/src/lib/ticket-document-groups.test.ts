import { describe, expect, it } from 'vitest'
import { groupTicketsByDocument, withoutDocumentReference, documentMembership } from './ticket-document-groups'
import type { Ticket } from './board'
const ref = (section_id: string, primary: boolean) => ({ id: section_id, section_id, title: section_id, primary, provenance: 'manual' as const, missing: false })
describe('document ticket grouping', () => {
  it('uses optional primary membership without duplicating secondary links or changing epic relationships', () => {
    const tickets: Ticket[] = [{ id: 'one', title: 'One', project: 'p', state: 'todo', parent: 'epic', document_refs: [ref('first', true), ref('second', false)] }, { id: 'two', title: 'Two', project: 'p', state: 'todo', document_refs: [ref('second', false)] }, { id: 'three', title: 'Three', project: 'p', state: 'todo' }]
    const groups = groupTicketsByDocument(tickets)
    expect([...groups.keys()]).toEqual(['first', ''])
    expect([...groups.values()].flat().map(ticket => ticket.id)).toEqual(['one', 'two', 'three'])
    expect(tickets[0]!.parent).toBe('epic')
    expect(tickets.filter(withoutDocumentReference).map(ticket => ticket.id)).toEqual(['three'])
  })
  it('treats deleted and previous-project references as unlinked, including a missing primary', () => {
    const tickets: Ticket[] = [{ id: 'old', title: 'Old', project: 'new', state: 'todo', document_refs: [{ ...ref('deleted', true), missing: true }] }, { id: 'mixed', title: 'Mixed', project: 'new', state: 'todo', document_refs: [{ ...ref('previous-project', true), missing: true }, ref('valid-secondary', false)] }]
    expect(tickets.filter(withoutDocumentReference).map(ticket => ticket.id)).toEqual(['old'])
    expect([...groupTicketsByDocument(tickets).keys()]).toEqual([''])
  })

  it('counts section membership once when original-source and related records name the same heading', () => {
    const ticket: Ticket = { id: 'one', title: 'One', project: 'p', state: 'todo', document_refs: [{ ...ref('same', false), id: 'source', provenance: 'direct' }, { ...ref('same', true), id: 'manual' }, ref('secondary', false)] }
    expect(documentMembership(ticket).map(reference => reference.id)).toEqual(['manual', 'secondary'])
  })

})
