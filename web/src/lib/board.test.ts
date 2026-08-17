// The event poll's reading of a page — the regression that stopped the board
// live-updating.
//
// `api<EventPage>` is an unchecked cast of a JSON body, so a type naming a field
// the server never sends compiles, typechecks, and lies at runtime. `EventPage`
// said `items`; the server sends `events`. The poll therefore read `undefined`
// every time, concluded nothing had happened, and never refreshed — while still
// setting the connection to "live", so the board showed stale tickets under a
// green dot indefinitely.
//
// The payloads below are the shape `/events` documents in spec/openapi.yaml,
// written out literally rather than built from the type under test: a fixture
// derived from the wrong type would have agreed with the bug.
import { afterEach, describe, expect, it, vi } from 'vitest'
import { getEvents, hasEvents } from './board'

afterEach(() => {
  vi.unstubAllGlobals()
})

function mockFetch(body: unknown) {
  const spy = vi.fn().mockResolvedValue(new Response(JSON.stringify(body), { status: 200 }))
  vi.stubGlobal('fetch', spy)
  return spy
}

const WIRE = {
  cursor: 107,
  events: [
    {
      seq: 107,
      kind: 'initiative_entry_added',
      project: 'demo',
      ticket: null,
      actor: 'human:me',
      at: '2026-08-17T16:31:00.000Z',
      payload: { initiative: 'ini-x', entry: 'ie-y' },
    },
  ],
}

describe('getEvents', () => {
  it('reads the array the server actually sends', async () => {
    mockFetch(WIRE)
    const page = await getEvents('tk_x', 0)
    expect(page.cursor).toBe(107)
    expect(page.events).toHaveLength(1)
    expect(page.events[0]?.kind).toBe('initiative_entry_added')
  })

  it('asks for events after the cursor it was given', async () => {
    const spy = mockFetch(WIRE)
    await getEvents('tk_x', 42)
    expect(String(spy.mock.calls[0]?.[0])).toContain('since=42')
  })
})

describe('hasEvents', () => {
  it('is true when the page carried anything', () => {
    expect(hasEvents(WIRE)).toBe(true)
  })

  it('is false for an empty page — a quiet poll must not trigger a refetch', () => {
    expect(hasEvents({ cursor: 107, events: [] })).toBe(false)
  })

  // What the bug looked like: every field the reader wanted was missing, and
  // "nothing happened" was indistinguishable from "I looked in the wrong place".
  it('is false when the array is missing entirely', () => {
    expect(hasEvents({ cursor: 107 } as unknown as Parameters<typeof hasEvents>[0])).toBe(false)
    expect(hasEvents(undefined)).toBe(false)
  })
})
