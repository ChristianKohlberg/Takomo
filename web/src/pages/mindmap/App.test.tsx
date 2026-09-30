// From a thought on the map straight to its section in the document.
//
// The right-click menu (reached here by Shift+F10, which opens the same menu on
// the selected node), ⌘K and the phone outline's `⋯` all offer "Open in
// document" and "Focus in document", to writers and readers alike, and each
// lands on the document view with `section=` — plus `focus=` for the second.
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { MemoryRouter, useLocation } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import * as Y from 'yjs'

import { createNode } from '@/lib/mindmap-crdt'
import { SpecificationContext, type SpecificationState } from '../specification/context'
import { MapView } from './App'

vi.mock('@/components/Toaster', () => ({ useToast: () => ({ toast: vi.fn() }) }))

function Where() {
  const location = useLocation()
  return <output data-testid="where">{location.pathname + location.search}</output>
}

let ydoc: Y.Doc
let nodeId: string

function mount(canWrite: boolean) {
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
    scopes: canWrite ? ['read', 'write'] : ['read'],
    voice: false,
    map,
    session: {
      object: map.id,
      mindmap: map.id,
      kind: 'mindmap',
      session: 'session',
      token: 'ticket',
      can_write: canWrite,
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

/** Shift+F10 on the canvas opens the node menu on the selected node. */
async function openNodeMenu() {
  // The selection arrives from `section=` once the node is read from the doc.
  await waitFor(() => expect(where()).toContain(`section=${nodeId}`))
  const canvas = document.querySelector('svg[tabindex]') as SVGElement
  expect(canvas).toBeTruthy()
  let menu: HTMLElement | null = null
  await waitFor(() => {
    fireEvent.keyDown(canvas, { key: 'F10', shiftKey: true })
    menu = screen.getByRole('menu', { name: 'This thought' })
  })
  return menu!
}

beforeEach(() => {
  localStorage.clear()
  ydoc = new Y.Doc()
  nodeId = createNode(ydoc, { parent: null, title: 'Billing', by: 'Someone' })!
})
afterEach(() => {
  cleanup()
  ydoc.destroy()
})

describe.each([
  ['writer', true],
  ['reader', false],
])('the node menu for a %s', (_who, canWrite) => {
  it('offers both document entries first, above every other verb', async () => {
    mount(canWrite)
    const items = within(await openNodeMenu()).getAllByRole('menuitem').map((item) => item.textContent)
    expect(items.slice(0, 2)).toEqual(['Open in document', 'Focus in document'])
    if (canWrite) expect(items).toContain('Rename this thought')
    else expect(items).not.toContain('Rename this thought')
  })

  it('opens the document at the section', async () => {
    mount(canWrite)
    const menu = await openNodeMenu()
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Open in document' }))
    await waitFor(() =>
      expect(where()).toBe(`/projects/vetbill/specification?view=document&section=${nodeId}`),
    )
  })

  it('opens the document focused on the section', async () => {
    mount(canWrite)
    const menu = await openNodeMenu()
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Focus in document' }))
    await waitFor(() =>
      expect(where()).toBe(
        `/projects/vetbill/specification?view=document&section=${nodeId}&focus=${nodeId}`,
      ),
    )
  })
})

describe('the command palette', () => {
  it('offers both entries for the selected thought and runs them', async () => {
    mount(false)
    await waitFor(() => expect(where()).toContain(`section=${nodeId}`))
    await waitFor(() => {
      fireEvent.keyDown(window, { key: 'k', ctrlKey: true })
      expect(screen.getByRole('dialog')).toBeTruthy()
    })
    const dialog = screen.getByRole('dialog')
    expect(within(dialog).getByText('Open in document')).toBeTruthy()
    // With a thought in scope "Read it as the plan" would be the same jump twice.
    expect(within(dialog).queryByText('Read it as the plan')).toBeNull()
    fireEvent.click(within(dialog).getByText('Focus in document'))
    await waitFor(() =>
      expect(where()).toBe(
        `/projects/vetbill/specification?view=document&section=${nodeId}&focus=${nodeId}`,
      ),
    )
  })
})

describe('the outline row menu', () => {
  it('offers both entries to a reader and opens the document at the section', async () => {
    mount(false)
    await waitFor(() => expect(where()).toContain(`section=${nodeId}`))
    fireEvent.click((await screen.findAllByRole('button', { name: 'Actions for this thought' }))[0]!)
    fireEvent.click(await screen.findByRole('button', { name: 'Open in document' }))
    await waitFor(() =>
      expect(where()).toBe(`/projects/vetbill/specification?view=document&section=${nodeId}`),
    )
  })
})
