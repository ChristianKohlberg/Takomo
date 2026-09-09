// Real browser against the leased server; never starts a substitute API server.
import { chromium } from 'playwright'
import { execFileSync } from 'node:child_process'
import { fileURLToPath, URL } from 'node:url'
const [base,db] = process.argv.slice(2)
if (!base || !db) throw new Error('usage: backlot-smoke.mjs <leased URL> <leased SQLite path>')
const root = fileURLToPath(new URL('../../',import.meta.url))
execFileSync('./target/release/takomo',['--db',db,'seed','--preset','dev'],{cwd:root,stdio:'pipe'})
const token = execFileSync('scripts/backlot-token.sh',['human',db],{cwd:root,encoding:'utf8'}).trim()
const browser = await chromium.launch({headless:true})
try {
  const page = await browser.newPage()
  const errors = []
  page.on('pageerror',e=>errors.push(e.message))
  await page.addInitScript(token=>{
    globalThis.localStorage.setItem('takomo.token',token)
    globalThis.localStorage.setItem('takomo.project','demo')
  },token)
  const response = await page.goto(new URL('/board',base).href)
  if (response?.status() !== 200) throw new Error('Board document failed')
  const projectResponse = await page.request.get(new URL('/v1/projects',base).href,{headers:{Authorization:`Bearer ${token}`}})
  if (!projectResponse.ok() || !(await projectResponse.json()).some(p=>p.id==='demo')) throw new Error('Seeded demo project missing')
  const tickets = await page.request.get(new URL('/v1/tickets?project=demo',base).href,{headers:{Authorization:`Bearer ${token}`}})
  const {items} = await tickets.json()
  if (!tickets.ok() || !items?.length) throw new Error('Seeded tickets missing')
  await page.getByText(items[0].title,{exact:true}).first().waitFor({state:'visible',timeout:20000})
  if (errors.length) throw new Error(errors.join('\n'))
  console.log('PASS: leased API authenticated; seeded ticket rendered in browser; no page errors')
} finally { await browser.close() }
