// The two ways the Map view could lose the specification in one move, guarded.
//
// Deleting the whole specification is offered to admins only, behind a
// two-step dialog that wants the id or title typed, and the request carries
// `confirm_id`. And a section with structure is never put in the plain notes
// box: the node dialog previews it and hands over to the document view.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { MemoryRouter, useLocation } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import * as Y from 'yjs'

import { createNode, proseOf, setNotes } from '@/lib/mindmap-crdt'
import { SpecificationContext, type SpecificationState } from '../specification/context'
import { MapView } from './App'
import { STR } from './strings'

// Read from the table, so rewording a label does not break what this pins.
const t = STR.en

vi.mock('@/components/Toaster', () => ({ useToast: () => ({ toast: vi.fn() }) }))

function Where() {
  const location = useLocation()
  return <output data-testid="where">{location.pathname + location.search}</output>
}

let ydoc: Y.Doc
let nodeId: string

function mount(scopes: string[]) {
  const provider = {
    synced: true,
    on: vi.fn(),
    off: vi.fn(),
    awareness: {
      clientID: 1,
      getStates: () => new Map(),
      on: vi.fn(),
      off: vi.fn(),
      setLocalStateField: vi.fn(),
    },
  }
  const map = { id: 'mm-1', project: 'vetbill', title: 'VetBill', status: 'open', nodes: 1, created_at: '' }
  const state = {
    token: 'token',
    lang: 'en',
    project: 'vetbill',
    projects: [{ id: 'vetbill', name: 'VetBill' }],
    actor: 'someone',
    scopes,
    voice: false,
    map,
    session: {
      object: map.id,
      mindmap: map.id,
      kind: 'mindmap',
      session: 'session',
      token: 'ticket',
      can_write: scopes.includes('write'),
      display: 'Someone',
      expires_at: '',
      url: '/sync',
      room: map.id,
    },
    connection: { ydoc, provider },
    nodes: [],
    verification: null,
    refreshMap: async () => null,
    refreshVerification: async () => null,
    selectProject: () => {},
    onError: () => {},
    openTests: () => {},
    openBehavior: () => {},
    testsFor: () => ({ total: 0, failing: 0, verified: 0 }),
  } as unknown as SpecificationState
  return render(
    <MemoryRouter initialEntries={[`/projects/vetbill/specification?view=map&section=${nodeId}`]}>
      <SpecificationContext.Provider value={state}>
        <MapView />
        <Where />
      </SpecificationContext.Provider>
    </MemoryRouter>,
  )
}

const where = () => screen.getByTestId('where').textContent

async function openPalette() {
  await waitFor(() => expect(where()).toContain(`section=${nodeId}`))
  await waitFor(() => {
    fireEvent.keyDown(window, { key: 'k', ctrlKey: true })
    expect(screen.getByRole('dialog')).toBeTruthy()
  })
  return screen.getByRole('dialog')
}

beforeEach(() => {
  localStorage.clear()
  ydoc = new Y.Doc()
  nodeId = createNode(ydoc, { parent: null, title: 'Billing', by: 'Someone' })!
})
afterEach(() => {
  cleanup()
  ydoc.destroy()
  vi.unstubAllGlobals()
})

describe('deleting the specification', () => {
  it('is not offered to a writer who is not an admin', async () => {
    mount(['read', 'write'])
    const palette = await openPalette()
    expect(within(palette).getByText(t.cmdRenameMap)).toBeTruthy()
    expect(within(palette).queryByText(t.cmdDeleteMap)).toBeNull()
  })

  it('takes an admin through two steps and a typed confirmation', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ ok: true, removed_nodes: 1 }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }),
    )
    vi.stubGlobal('fetch', fetchMock)
    const confirm = vi.spyOn(window, 'confirm')
    mount(['read', 'write', 'admin'])
    const palette = await openPalette()
    fireEvent.click(within(palette).getByText(t.cmdDeleteMap))

    await screen.findByText(t.deleteSpecFirst)
    fireEvent.click(screen.getByRole('button', { name: t.deleteSpecContinue }))
    const final = screen.getByRole('button', { name: t.deleteSpecFinal }) as HTMLButtonElement
    expect(final.disabled).toBe(true)
    fireEvent.change(screen.getByLabelText(t.deleteSpecType), {
      target: { value: 'VetBill' },
    })
    expect(final.disabled).toBe(false)
    await act(async () => {
      fireEvent.click(final)
    })

    const call = fetchMock.mock.calls.find(([, init]) => (init as RequestInit)?.method === 'DELETE')
    expect(call).toBeTruthy()
    expect(String(call![0])).toContain('/mindmaps/mm-1')
    expect(JSON.parse(String((call![1] as RequestInit).body))).toEqual({ confirm_id: 'mm-1' })
    expect(confirm).not.toHaveBeenCalled()
  })
})

describe('the node dialog on a structured section', () => {
  it('offers the document instead of the plain notes box, and goes there', async () => {
    setNotes(ydoc, nodeId, 'The lead.')
    const frag = proseOf(ydoc, nodeId)!
    ydoc.transact(() => {
      const table = new Y.XmlElement('table')
      table.insert(0, [new Y.XmlText('Refunds')])
      frag.insert(frag.length, [table])
    })
    mount(['read', 'write'])
    const palette = await openPalette()
    fireEvent.click(within(palette).getByText(t.cmdOpen))

    const button = await screen.findByRole('button', { name: t.editInDocument })
    expect(screen.queryByRole('textbox', { name: t.notes })).toBeNull()
    fireEvent.click(button)
    await waitFor(() =>
      expect(where()).toBe(`/projects/vetbill/specification?view=document&section=${nodeId}`),
    )
  })

  it('keeps the plain notes box for a plain section', async () => {
    setNotes(ydoc, nodeId, 'Just words.')
    mount(['read', 'write'])
    const palette = await openPalette()
    fireEvent.click(within(palette).getByText(t.cmdOpen))
    // It opens on the read preview, one click away from the plain box.
    fireEvent.click(await screen.findByRole('button', { name: t.editNotes }))
    expect(screen.getByRole('textbox', { name: t.notes })).toBeTruthy()
    expect(screen.queryByRole('button', { name: t.editInDocument })).toBeNull()
  })
})
