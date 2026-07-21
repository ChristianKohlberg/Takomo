// In-memory fence (lease token) tracking.
//
// The store hands out a fencing token when you claim a ticket, and every
// state-changing call on a claimed ticket (transition, release) must echo it.
// Because the MCP server is a long-lived process for the duration of an agent
// session, we remember the fence per ticket id so the agent does not have to
// pass it back on every call. An explicit fence argument always overrides the
// remembered value.

export interface Lease {
  fence: number;
  holder?: string;
  expiresAt?: string;
}

const leases = new Map<string, Lease>();

export function rememberLease(ticketId: string, lease: Lease): void {
  leases.set(ticketId, lease);
}

export function getFence(ticketId: string): number | undefined {
  return leases.get(ticketId)?.fence;
}

export function getLease(ticketId: string): Lease | undefined {
  return leases.get(ticketId);
}

export function forgetLease(ticketId: string): void {
  leases.delete(ticketId);
}

// Resolve the fence to send: an explicit override wins, otherwise the
// remembered lease for the ticket (may be undefined if we never claimed it).
export function resolveFence(ticketId: string, override?: number): number | undefined {
  if (override !== undefined && override !== null) return override;
  return getFence(ticketId);
}
