import { StrictMode, useEffect, useState } from 'react'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { SpecificationViews } from './Views'
import { useSyncConnection } from '@/hooks/useSyncConnection'
import { usePersonalSelection } from '@/hooks/usePersonalSelection'
import type { SpecificationView } from '@/lib/specification-url'

const providers = vi.hoisted(() => [] as { destroyed: boolean }[])
vi.mock('y-websocket', () => ({
  WebsocketProvider: class {
    destroyed = false
    constructor() {
      providers.push(this)
    }
    destroy() {
      this.destroyed = true
    }
  },
}))
vi.mock('@/lib/save-status', () => ({
  trackSave: () => ({ ready: Promise.resolve(), destroy: vi.fn() }),
}))
vi.mock('@/lib/renew-session', () => ({ renewSession: () => () => {} }))
const session = {
  object: 'mm-test',
  room: 'mm-test',
  token: 'ticket',
  expires_at: '',
  url: '/v1/docsync',
}
afterEach(() => {
  cleanup()
  providers.length = 0
})

it('preserves view state and the shared replica while pausing hidden view effects', async () => {
  const active = new Set<string>()
  const selection = vi.fn()
  function View({ name }: { name: string }) {
    const [draft, setDraft] = useState('')
    const [, select] = usePersonalSelection(selection)
    useEffect(() => {
      active.add(name)
      return () => {
        active.delete(name)
      }
    }, [name])
    return (
      <>
        <input aria-label={name} value={draft} onChange={(event) => setDraft(event.target.value)} />
        <button onClick={() => select(name)}>Select {name}</button>
      </>
    )
  }
  function Workspace({ view }: { view: SpecificationView }) {
    const connection = useSyncConnection(session, () => {})
    return (
      <>
        <output>{connection ? 'connected' : 'loading'}</output>
        <SpecificationViews
          current={view}
          loading="loading"
          views={{
            document: <View name="document" />,
            map: <View name="map" />,
            tests: <View name="tests" />,
          }}
        />
      </>
    )
  }
  const ui = render(
    <StrictMode>
      <Workspace view="document" />
    </StrictMode>,
  )
  await screen.findByText('connected')
  const provider = providers.at(-1)
  fireEvent.change(screen.getByLabelText('document'), { target: { value: 'Unfinished paragraph' } })
  fireEvent.click(screen.getByText('Select document'))
  ui.rerender(
    <StrictMode>
      <Workspace view="map" />
    </StrictMode>,
  )
  await waitFor(() => expect([...active]).toEqual(['map']))
  fireEvent.click(screen.getByText('Select map'))
  ui.rerender(
    <StrictMode>
      <Workspace view="tests" />
    </StrictMode>,
  )
  await waitFor(() => expect([...active]).toEqual(['tests']))
  ui.rerender(
    <StrictMode>
      <Workspace view="document" />
    </StrictMode>,
  )
  await waitFor(() => expect([...active]).toEqual(['document']))
  expect((screen.getByLabelText('document') as HTMLInputElement).value).toBe('Unfinished paragraph')
  expect(selection.mock.calls).toEqual([['document'], ['map']])
  expect(providers.filter((value) => !value.destroyed)).toEqual([provider])
  ui.unmount()
  expect(provider?.destroyed).toBe(true)
})
