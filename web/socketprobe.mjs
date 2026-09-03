import * as Y from 'yjs'
import { WebsocketProvider } from 'y-websocket'
import WS from 'ws'
import fs from 'node:fs'

const t = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'))
const BASE = 'http://127.0.0.1:8499'
const doc = new Y.Doc()
const provider = new WebsocketProvider(BASE.replace('http', 'ws') + t.url, t.room, doc, {
  params: { ticket: t.token },
  WebSocketPolyfill: WS,
})
const synced = await new Promise((res) => {
  const timer = setTimeout(() => res(false), 12000)
  provider.on('sync', (ok) => { if (ok) { clearTimeout(timer); res(true) } })
})
console.log('  socket synced:', synced)
if (!synced) { provider.destroy(); process.exit(0) }

const nodes = doc.getMap('nodes')
const entry = nodes.get(process.argv[3])
const before = entry.get('title')
doc.transact(() => entry.set('title', 'WRITTEN WHILE ARCHIVED'))
console.log('  wrote locally, was:', JSON.stringify(before))
await new Promise((r) => setTimeout(r, 3000))
provider.destroy()

const res = await fetch(`${BASE}/v1/mindmaps/${t.object}`, {
  headers: { Authorization: `Bearer ${process.argv[4]}` },
})
const body = await res.json()
const n = (body.nodes || []).find((x) => x.id === process.argv[3])
console.log('  server now reads title:', JSON.stringify(n && n.text))
