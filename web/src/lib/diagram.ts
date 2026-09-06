import { createContext } from 'react'
import { api } from './api'
import { defineStrings, detectLocale } from './i18n'

export type DiagramEngine = 'mermaid' | 'plantuml' | 'd2'
export interface DiagramAccess { token: string; project: string }
export const DiagramContext = createContext<DiagramAccess | null>(null)
export function diagramEngine(language: unknown): DiagramEngine | null {
  switch (String(language ?? '').toLowerCase()) {
    case 'mermaid': return 'mermaid'
    case 'plantuml': case 'puml': case 'salt': return 'plantuml'
    case 'd2': return 'd2'
    default: return null
  }
}
const strings = defineStrings({
  en: { title: 'Diagram', loading: 'Rendering diagram…', outdated: 'Preview is outdated.', large: 'Diagram is too large to render. Select Code to view the source.', error: 'Unable to render diagram.', access: 'Select a project and sign in to render diagrams.', retry: 'Retry' },
  de: { title: 'Diagramm', loading: 'Diagramm wird gerendert…', outdated: 'Die Vorschau ist veraltet.', large: 'Das Diagramm ist zu groß. Wähle Code, um den Quelltext zu sehen.', error: 'Das Diagramm konnte nicht gerendert werden.', access: 'Wähle ein Projekt und melde dich an, um Diagramme zu rendern.', retry: 'Erneut versuchen' },
})
function labels() {
  let locale: string | null = null
  try { locale = localStorage.getItem('takomo.lang') } catch { /* Browser locale is the fallback. */ }
  return strings[detectLocale(locale)]
}
function imageFromSvg(svg: string): HTMLImageElement {
  const root = new DOMParser().parseFromString(svg, 'image/svg+xml').documentElement
  if (root.localName !== 'svg' || root.namespaceURI !== 'http://www.w3.org/2000/svg') throw new Error(labels().error)
  // SVG is an isolated image document, never markup in the application's DOM.
  const image = document.createElement('img')
  image.alt = labels().title
  const viewBox = root.getAttribute('viewBox')?.trim().split(/[\s,]+/).map(Number)
  if (viewBox?.length === 4 && viewBox.every(Number.isFinite) && viewBox[2]! > 0 && viewBox[3]! > 0) {
    image.width = Math.ceil(viewBox[2]!)
    image.height = Math.ceil(viewBox[3]!)
  } else {
    const width = Number.parseFloat(root.getAttribute('width') ?? '')
    const height = Number.parseFloat(root.getAttribute('height') ?? '')
    if (Number.isFinite(width) && Number.isFinite(height) && width > 0 && height > 0) {
      image.width = Math.ceil(width); image.height = Math.ceil(height)
    }
  }
  image.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`
  image.style.backgroundColor = 'white'
  image.style.maxWidth = '100%'
  image.style.height = 'auto'
  return image
}

// A document can contain many blocks. Share a small request pool so opening it
// does not flood the rendering service; cancelled queued work never starts.
let active = 0
const waiting: (() => void)[] = []
function renderingSlot(signal: AbortSignal): Promise<() => void> {
  return new Promise((resolve, reject) => {
    const abort = () => {
      const index = waiting.indexOf(start)
      if (index !== -1) waiting.splice(index, 1)
      reject(new DOMException('Cancelled', 'AbortError'))
    }
    const start = () => {
      signal.removeEventListener('abort', abort)
      if (signal.aborted) { abort(); return }
      active++
      resolve(() => { active--; waiting.shift()?.() })
    }
    if (signal.aborted) { abort(); return }
    if (active < 2) start()
    else { waiting.push(start); signal.addEventListener('abort', abort, { once: true }) }
  })
}
async function renderSvg(access: DiagramAccess, source: string, engine: DiagramEngine, signal: AbortSignal) {
  const release = await renderingSlot(signal)
  try {
    if (signal.aborted) throw new DOMException('Cancelled', 'AbortError')
    return await api<{ svg: string }>(access.token, '/diagrams/render', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ project: access.project, engine, source }), signal,
    })
  } finally { release() }
}

/** Debounced, cancellable service rendering. The last good preview survives edits/errors. */
export function mountDiagram(host: HTMLElement, source: string, engine: DiagramEngine = 'mermaid', access: DiagramAccess | null = null): () => void {
  let cancelled = false
  const controller = new AbortController()
  const t = labels()
  const previous = host.querySelector('img')
  const status = document.createElement('div')
  status.setAttribute('role', 'status')
  const setStatus = (message: string) => { status.textContent = `${previous ? `${t.outdated} ` : ''}${message}` }
  host.replaceChildren(...(previous ? [previous, status] : [status]))
  setStatus(t.loading)
  if (!access?.token || !access.project || new TextEncoder().encode(source).length > 50_000) {
    setStatus(!access?.token || !access.project ? t.access : t.large)
    return () => { cancelled = true }
  }
  const render = () => {
    setStatus(t.loading)
    void renderSvg(access, source, engine, controller.signal).then(({ svg }) => {
      if (!cancelled) host.replaceChildren(imageFromSvg(svg))
    }).catch((error: unknown) => {
      if (cancelled) return
      setStatus(`${t.error} ${error instanceof Error ? error.message : ''}`)
      const retry = document.createElement('button')
      retry.type = 'button'
      retry.textContent = t.retry
      retry.addEventListener('click', render, { once: true })
      status.append(' ', retry)
    })
  }
  const timer = setTimeout(render, 250)
  return () => { cancelled = true; clearTimeout(timer); controller.abort() }
}
