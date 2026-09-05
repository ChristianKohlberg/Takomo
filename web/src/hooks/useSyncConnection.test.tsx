import { StrictMode } from 'react'
import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { useSyncConnection } from './useSyncConnection'

const providers = vi.hoisted(() => [] as { destroyed: boolean; options: { disableBc: boolean; connect: boolean } }[])
vi.mock('y-websocket', () => ({ WebsocketProvider: class {
  destroyed = false
  constructor(_url: string, _room: string, _doc: unknown, public options: { disableBc: boolean; connect: boolean }) { providers.push(this) }
  destroy() { this.destroyed = true }
} }))
vi.mock('@/lib/renew-session', () => ({ renewSession: () => () => {} }))
const session = { object: 'mm-test', room: 'mm-test', token: 'ticket', expires_at: '', url: '/v1/docsync' }
function Probe({ room = session }: { room?: typeof session }) {
  const connection = useSyncConnection(room, () => {})
  return <div>{connection ? String(Object.is(connection.provider, providers.at(-1))) : 'loading'}</div>
}
afterEach(() => { cleanup(); providers.length = 0 })
it('owns one live connection after StrictMode replay and destroys every discarded connection', () => {
  const view = render(<StrictMode><Probe /></StrictMode>)
  expect(screen.getByText('true')).toBeTruthy()
  expect(providers.filter(p => !p.destroyed)).toHaveLength(1)
  expect(providers.every(p => p.options.disableBc && !p.options.connect)).toBe(true)
  view.rerender(<StrictMode><Probe /></StrictMode>)
  expect(providers.filter(p => !p.destroyed)).toHaveLength(1)
  view.unmount()
  expect(providers.every(p => p.destroyed)).toBe(true)
})
it('releases the old replica when switching rooms', () => {
  const view = render(<Probe />)
  const first = providers[0]!
  view.rerender(<Probe room={{ ...session, room: 'mm-other', object: 'mm-other' }} />)
  expect(first.destroyed).toBe(true)
  expect(providers.filter(p => !p.destroyed)).toHaveLength(1)
})
