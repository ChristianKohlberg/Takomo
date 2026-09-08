import type { Ticket } from './board'
/** Source and related records can refer to the same section; display membership once. */
export function documentMembership(ticket: Ticket) {
  const sections = new Map<string, NonNullable<Ticket['document_refs']>[number]>()
  for (const reference of ticket.document_refs ?? []) {
    if (!reference.missing && (!sections.has(reference.section_id) || reference.primary)) sections.set(reference.section_id, reference)
  }
  return [...sections.values()]
}
export function withoutDocumentReference(ticket: Ticket) { return !(ticket.document_refs?.some(reference => !reference.missing)) }
/** A secondary reference never duplicates a ticket into another heading group. */
export function groupTicketsByDocument(tickets: readonly Ticket[]) {
  const groups = new Map<string, Ticket[]>()
  for (const ticket of tickets) {
    const key = ticket.document_refs?.find(reference => reference.primary && !reference.missing)?.section_id ?? ''
    const group = groups.get(key) ?? []; group.push(ticket); groups.set(key, group)
  }
  return groups
}
