// Real-browser measurement of outline navigation in the specification document.
//
//   node scripts/document-scroll-measure.mjs seed    <base URL> <token> <project>
//   node scripts/document-scroll-measure.mjs measure <base URL> <token> <project> [out.json] [screenshot dir]
//
// `seed` fills the project's (empty) specification with a large document shaped
// like a real audit spec: 56 sections, long prose, tables well past the long-table
// collapse, and mermaid/dbml diagrams. Prose goes in the way an agent writes it —
// a proposal accepted server-side — after one seed paragraph per section written
// over the sync socket, because an empty section has no block to insert after.
//
// `measure` goes to ten sections spread across the document four ways — an
// outline click, a cold page load with `section=`, an in-app navigation to a new
// `section=`, and an outline click inside a section focus — and records how far
// the target's section top is from the top of the scroll column at 100 ms,
// 500 ms, 1 s, 2 s and 4 s after the click or navigation (for a cold load: after
// the section first exists). Zero is the resting position (`docs/documents.md`,
// "Getting to a section").
// Needs Chromium: `npx playwright install chromium`.
/* global document, performance, requestAnimationFrame, history, dispatchEvent, PopStateEvent, fetch -- sampleInPage and the navigation run in the page */
import { chromium } from 'playwright'
import { setTimeout, clearTimeout } from 'node:timers'
import { URL } from 'node:url'
import { WebsocketProvider } from 'y-websocket'
import * as Y from 'yjs'
import { mkdirSync, writeFileSync } from 'node:fs'

const [mode, base, token, project, out, shots] = process.argv.slice(2)
if (!['seed', 'measure'].includes(mode) || !base || !token || !project) {
  throw new Error('usage: document-scroll-measure.mjs seed|measure <base URL> <token> <project> [out.json] [screenshot dir]')
}
const TIMES = [100, 500, 1000, 2000, 4000]

async function api(path, init = {}) {
  let response
  // The seed writes more than a token's per-minute write budget; wait it out.
  for (let attempt = 0; ; attempt++) {
    response = await fetch(new URL(`/v1${path}`, base), {
      ...init,
      headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json', ...init.headers },
    })
    if (response.status !== 429 || attempt >= 40) break
    await new Promise(resolve => setTimeout(resolve, 5000))
  }
  const body = await response.json().catch(() => null)
  if (!response.ok) throw new Error(`${init.method ?? 'GET'} ${path}: ${response.status} ${JSON.stringify(body)}`)
  return body
}

async function specification() {
  const { items } = await api(`/mindmaps?project=${encodeURIComponent(project)}`)
  return items[0] ?? null
}

// ---- seed ------------------------------------------------------------------

const WORDS = 'the audit records each finding against the branch and the checklist item that raised it so a reviewer can trace every deviation back to its evidence and the person who confirmed it before the report is signed off and archived'.split(' ')
function sentence(seed, length) {
  const out = []
  for (let i = 0; i < length; i++) out.push(WORDS[(seed * 7 + i * 13) % WORDS.length])
  const text = out.join(' ')
  return text[0].toUpperCase() + text.slice(1) + '.'
}
function paragraph(seed, sentences) {
  return Array.from({ length: sentences }, (_, i) => sentence(seed + i, 14 + ((seed + i) % 9))).join(' ')
}
function table(seed, rows) {
  const head = '| Code | Check | Owner | Evidence |\n| --- | --- | --- | --- |'
  const body = Array.from({ length: rows }, (_, i) => `| C-${seed}-${i + 1} | ${sentence(seed + i, 6)} | Team ${(i % 4) + 1} | ${sentence(seed + i * 3, 9)} |`)
  return [head, ...body].join('\n')
}
const MERMAID = n => `\`\`\`mermaid\nflowchart TD\n  A${n}[Finding] --> B${n}{Severity}\n  B${n} -->|high| C${n}[Escalate]\n  B${n} -->|low| D${n}[Record]\n  C${n} --> E${n}[Report]\n  D${n} --> E${n}\n\`\`\``
const DBML = n => `\`\`\`dbml\nTable audit_${n} {\n  id int [pk]\n  branch varchar\n  checklist int\n  score decimal\n}\nTable finding_${n} {\n  id int [pk]\n  audit int [ref: > audit_${n}.id]\n  text varchar\n}\n\`\`\``

function sectionMarkdown(i) {
  const parts = [paragraph(i, 4 + (i % 4)), paragraph(i + 50, 3)]
  if (i % 4 === 1) parts.push(table(i, 12 + (i % 5) * 7))
  if (i % 9 === 2) parts.push(MERMAID(i))
  if (i % 11 === 5) parts.push(DBML(i))
  if (i % 3 === 0) parts.push(paragraph(i + 100, 6), paragraph(i + 150, 5))
  if (i % 7 === 3) parts.push(table(i + 1000, 11))
  return parts.join('\n\n')
}

/** Section titles in document order: the order `seed` creates them in. */
const TITLES = Array.from({ length: 8 }, (_, c) => [
  `Chapter ${c + 1}: audit area ${c + 1}`,
  ...Array.from({ length: 5 }, (_, k) => [`Topic ${c + 1}.${k + 1}`, ...(k === 1 ? [`Detail ${c + 1}.${k + 1}.1`] : [])]).flat(),
]).flat()

async function seed() {
  const existing = await specification()
  if (existing) {
    if (existing.title !== 'Audit specification (scroll fixture)') throw new Error(`project ${project} already has another specification; seed an empty project`)
    return fill(existing.id)
  }
  const map = await api('/mindmaps', { method: 'POST', body: JSON.stringify({ project, title: 'Audit specification (scroll fixture)' }) })
  const id = map.id ?? map.mindmap?.id
  // 8 chapters × (1 + 6 children), the second child of each with a grandchild: 64 → trim to 56.
  const nodes = []
  const chapters = (await api(`/mindmaps/${id}/nodes`, { method: 'POST', body: JSON.stringify({ nodes: Array.from({ length: 8 }, (_, c) => ({ title: `Chapter ${c + 1}: audit area ${c + 1}` })) }) })).nodes
  for (const [c, chapter] of chapters.entries()) {
    nodes.push(chapter.id)
    const children = (await api(`/mindmaps/${id}/nodes`, { method: 'POST', body: JSON.stringify({ nodes: Array.from({ length: 5 }, (_, k) => ({ parent: chapter.id, title: `Topic ${c + 1}.${k + 1}` })) }) })).nodes
    for (const [k, child] of children.entries()) {
      nodes.push(child.id)
      if (k === 1) {
        const [grand] = (await api(`/mindmaps/${id}/nodes`, { method: 'POST', body: JSON.stringify({ parent: child.id, title: `Detail ${c + 1}.${k + 1}.1` }) })).nodes
        nodes.push(grand.id)
      }
    }
  }
  // One seed paragraph per section, written over the sync socket.
  const session = await api(`/mindmaps/${id}/session`, { method: 'POST', body: '{}' })
  const ydoc = new Y.Doc()
  const wsBase = new URL(session.url, base.replace(/^http/, 'ws')).href
  const provider = new WebsocketProvider(wsBase, session.room, ydoc, { params: { ticket: session.token } })
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('sync timed out')), 15000)
    provider.on('sync', synced => { if (synced) { clearTimeout(timer); resolve() } })
  })
  const map2 = ydoc.getMap('nodes')
  ydoc.transact(() => {
    nodes.forEach((node, i) => {
      const prose = map2.get(node)?.get('prose')
      if (!prose) throw new Error(`node ${node} has no prose fragment`)
      const p = new Y.XmlElement('paragraph')
      p.setAttribute('id', `blk_seed_${i}`)
      p.insert(0, [new Y.XmlText(`Section ${i + 1} opens here.`)])
      prose.insert(0, [p])
    })
  })
  await new Promise(resolve => setTimeout(resolve, 3000))
  provider.destroy()
  ydoc.destroy()
  return fill(id)
}

/** Writes each section's body; resumable, since only still-empty sections are written. */
async function fill(id) {
  const map = await api(`/mindmaps/${id}`)
  const byTitle = new Map(map.nodes.map(node => [node.title ?? node.text, node.id]))
  const pending = (await api(`/mindmaps/${id}/proposals?status=pending`)).items
  for (const [i, title] of TITLES.entries()) {
    const node = byTitle.get(title)
    const { markdown } = await api(`/mindmaps/${id}/prose?node=${node}`)
    if ((markdown.match(/<!-- blk_/g) ?? []).length > 1) continue
    const waiting = pending.find(proposal => proposal.node === node)
    const proposal = waiting?.id ?? (await api(`/mindmaps/${id}/proposals`, { method: 'POST', body: JSON.stringify({ node, operations: [{ op: 'insert_after', id: `blk_seed_${i}`, markdown: sectionMarkdown(i) }], summary: 'fixture' }) })).proposal
    await api(`/mindmaps/${id}/proposals/${proposal}/accept`, { method: 'POST', body: '{}' })
  }
  console.log(JSON.stringify({ mindmap: id, sections: TITLES.length }))
}

// ---- measure -----------------------------------------------------------------

/**
 * In the page: wait until `since` (a performance.now() stamp) — or, with none,
 * until the target section first exists, which is when a cold open can begin to
 * place it — then record the target's offset from the column top at each time.
 */
async function sampleInPage({ times, since, label }) {
  const column = () => document.querySelector('.document-page')?.parentElement
  const sectionOf = () => [...document.querySelectorAll('.document-section')].find(section => {
    const heading = section.querySelector('.document-heading')
    const number = section.querySelector('.document-section-number')?.textContent ?? ''
    return `${number ? `${number} ` : ''}${heading?.textContent ?? ''}` === label
  })
  let start = since
  if (start === null) {
    const deadline = performance.now() + 30000
    while (!sectionOf() && performance.now() < deadline) await new Promise(resolve => requestAnimationFrame(resolve))
    start = performance.now()
  }
  const out = []
  for (const at of times) {
    const wait = start + at - performance.now()
    if (wait > 0) await new Promise(resolve => setTimeout(resolve, wait))
    const section = sectionOf()
    const scroller = column()
    if (!scroller) { out.push({ at, offset: null, heading: null, scrollTop: null, clamped: false }); continue }
    const columnTop = scroller.getBoundingClientRect().top
    const heading = section?.querySelector('.document-heading')
    out.push({
      at,
      offset: section ? Math.round(section.getBoundingClientRect().top - columnTop) : null,
      heading: heading ? Math.round(heading.getBoundingClientRect().top - columnTop) : null,
      scrollTop: Math.round(scroller.scrollTop),
      clamped: scroller.scrollTop >= scroller.scrollHeight - scroller.clientHeight - 1,
    })
  }
  return out
}

async function measure() {
  const map = await specification()
  if (!map) throw new Error('no specification; run seed first')
  const browser = await chromium.launch({ headless: true })
  const results = { base, when: new Date().toISOString(), runs: [] }
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
    await page.addInitScript(([token, project]) => {
      globalThis.localStorage.setItem('takomo.token', token)
      globalThis.localStorage.setItem('takomo.project', project)
    }, [token, project])
    const url = search => new URL(`/projects/${project}/specification?view=document${search}`, base).href
    const open = async search => {
      await page.goto(url(search), { timeout: 120000 })
      await page.locator('[role=treeitem]').first().waitFor({ state: 'visible', timeout: 120000 })
      await page.waitForTimeout(1500)
    }
    await open('')
    const rows = await page.locator('[role=treeitem]').evaluateAll(items => items.map(item => ({ label: item.getAttribute('aria-label') })))
    const stats = await page.evaluate(() => {
      const column = document.querySelector('.document-page')?.parentElement
      return { sections: document.querySelectorAll('.document-section').length, scrollHeight: column?.scrollHeight }
    })
    // Section ids by label, for the URL paths.
    const ids = Object.fromEntries(await page.evaluate(() => [...document.querySelectorAll('.document-section')].map(section => [
      `${section.querySelector('.document-section-number')?.textContent ? `${section.querySelector('.document-section-number').textContent} ` : ''}${section.querySelector('.document-heading')?.textContent ?? ''}`,
      section.dataset.section,
    ])))
    results.document = { outlineEntries: rows.length, ...stats }
    // Ten targets spread over the first ~85% (the last screenful cannot reach the top).
    const pickFrom = (list, n) => Array.from({ length: n }, (_, i) => list[Math.round(1 + i * (list.length * 0.85 - 1) / (n - 1))]).filter(Boolean)
    // Scrambled order so consecutive jumps go both ways and cover long distances.
    const order = [5, 0, 9, 2, 7, 3, 8, 1, 6, 4]
    const targets = pickFrom(rows, 10)
    const shot = async (label, n) => { if (shots && n <= 3) await page.screenshot({ path: `${shots}/${label}-${n}.png` }) }
    const clicks = async (label, targetList) => {
      const samples = []
      for (const index of order) {
        const target = targetList[index % targetList.length]
        if (!target) continue
        const button = page.locator(`[role=treeitem][aria-label="${target.label}"]`).first().getByRole('button', { name: target.label, exact: true })
        try {
          await button.scrollIntoViewIfNeeded({ timeout: 30000 })
        } catch (error) {
          if (shots) await page.screenshot({ path: `${shots}/${label}-failed.png` })
          const items = await page.locator('[role=treeitem]').evaluateAll(list => list.map(item => item.getAttribute('aria-label')))
          throw new Error(`${target.label} not in the outline (${items.length} entries: ${items.slice(0, 8).join(', ')}…)`, { cause: error })
        }
        const since = await page.evaluate(() => performance.now())
        await button.click()
        samples.push({ target: target.label, series: await page.evaluate(sampleInPage, { times: TIMES, since, label: target.label }) })
        await shot(label, samples.length)
      }
      return samples
    }
    results.runs.push({ mode: 'outline click, no focus', samples: await clicks('nofocus', targets) })

    // A link opened cold: `?view=document&section=<id>` in a fresh page load.
    const cold = []
    for (const index of order) {
      const target = targets[index]
      await page.goto(url(`&section=${ids[target.label]}`), { waitUntil: 'domcontentloaded', timeout: 120000 })
      cold.push({ target: target.label, series: await page.evaluate(sampleInPage, { times: TIMES, since: null, label: target.label }) })
      await shot('cold', cold.length)
    }
    results.runs.push({ mode: 'cold open with section=', samples: cold })

    // In-app navigation to a new `section=` while the document is mounted: a
    // router history change, as a link from the map or Back/Forward makes.
    await open('')
    const warm = []
    for (const index of order) {
      const target = targets[index]
      const since = await page.evaluate(href => {
        const since = performance.now()
        history.pushState(history.state, '', href)
        dispatchEvent(new PopStateEvent('popstate', { state: history.state }))
        return since
      }, url(`&section=${ids[target.label]}`))
      warm.push({ target: target.label, series: await page.evaluate(sampleInPage, { times: TIMES, since, label: target.label }) })
      await shot('warm', warm.length)
    }
    results.runs.push({ mode: 'in-app navigation to section=', samples: warm })

    // Focus a chapter in the middle; its subtree is the outline now.
    const chapters = rows.filter(row => /^\d+ /.test(row.label ?? ''))
    const chapter = chapters[Math.ceil(chapters.length / 2) - 1]
    await open(`&focus=${ids[chapter.label]}`)
    const focusRows = await page.locator('[role=treeitem]').evaluateAll(items => items.map(item => ({ label: item.getAttribute('aria-label') })))
    // Inside a focus the whole subtree is short; every entry but the root is a target.
    results.runs.push({ mode: `outline click, focus on ${chapter.label}`, samples: await clicks('focus', focusRows.slice(1)) })
  } finally {
    await browser.close()
  }
  const summary = results.runs.map(run => {
    const settled = run.samples.filter(sample => sample.series.every(point => point.at < 500 || (point.offset !== null && (Math.abs(point.offset) <= 8 || point.clamped))))
    return `${run.mode}: ${settled.length}/${run.samples.length} within ±8 px from 500 ms to 4 s`
  })
  results.summary = summary
  if (out) writeFileSync(out, JSON.stringify(results, null, 2))
  console.log(summary.join('\n'))
}

if (mode === 'seed') await seed()
else {
  if (shots) mkdirSync(shots, { recursive: true })
  await measure()
}
