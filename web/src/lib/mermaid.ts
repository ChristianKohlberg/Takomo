import { defineStrings, detectLocale } from './i18n'

const strings = defineStrings({
  en: { title: 'Mermaid diagram', loading: 'Rendering diagram…', large: 'Diagram is too large to render. Select Code to view the source.', error: 'Unable to render Mermaid diagram. Select Code to check the source.' },
  de: { title: 'Mermaid-Diagramm', loading: 'Diagramm wird gerendert…', large: 'Das Diagramm ist zu groß. Wähle Code, um den Quelltext zu sehen.', error: 'Das Mermaid-Diagramm konnte nicht gerendert werden. Prüfe den Quelltext unter Code.' },
})
function labels() {
  let locale: string | null = null
  try { locale = localStorage.getItem('takomo.lang') } catch { /* Browser locale is the fallback. */ }
  return strings[detectLocale(locale)]
}

/** Lazy renderer shared by Markdown and the collaborative code-block view. */
let nextId = 0
let renderer: Promise<typeof import('mermaid')['default']> | undefined

async function renderImage(source: string): Promise<HTMLImageElement> {
  renderer ??= import('mermaid').then(({ default: mermaid }) => {
    mermaid.initialize({
      startOnLoad: false,
      securityLevel: 'strict',
      htmlLabels: false,
      flowchart: { htmlLabels: false },
      // Diagram directives/frontmatter must not weaken these settings.
      secure: ['secure', 'securityLevel', 'startOnLoad', 'maxTextSize', 'maxEdges', 'htmlLabels', 'flowchart', 'suppressErrorRendering'],
      maxTextSize: 50_000,
      maxEdges: 500,
      suppressErrorRendering: true,
    })
    return mermaid
  }).catch((error: unknown) => { renderer = undefined; throw error })
  const mermaid = await renderer
  const { svg } = await mermaid.render(`takomo-mermaid-${++nextId}`, source)
  // SVG remains an image document: scripts, links and foreign content cannot
  // execute in the application's DOM. The existing CSP permits data images.
  const image = document.createElement('img')
  image.alt = labels().title
  // Mermaid emits width=100%; an image needs intrinsic dimensions to avoid
  // enlarging a small diagram to the width of the whole document column.
  const root = new DOMParser().parseFromString(svg, 'image/svg+xml').documentElement
  const viewBox = root.getAttribute('viewBox')?.trim().split(/[\s,]+/).map(Number)
  if (viewBox?.length === 4 && viewBox.every(Number.isFinite) && viewBox[2]! > 0 && viewBox[3]! > 0) {
    image.width = Math.ceil(viewBox[2]!)
    image.height = Math.ceil(viewBox[3]!)
  }
  image.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`
  // Mermaid's default light palette needs a light canvas in dark app themes.
  image.style.backgroundColor = 'white'
  image.style.maxWidth = '100%'
  image.style.height = 'auto'
  return image
}

/** Returns cancellation for unmounts/edits; stale async results never win. */
export function mountMermaid(host: HTMLElement, source: string): () => void {
  let cancelled = false
  host.replaceChildren()
  host.setAttribute('role', 'status')
  host.textContent = labels().loading
  if (source.length > 50_000) {
    host.textContent = labels().large
    return () => { cancelled = true }
  }
  const timer = setTimeout(() => {
    void renderImage(source).then((image) => {
      if (!cancelled) host.replaceChildren(image)
    }).catch(() => {
      if (!cancelled) host.textContent = labels().error
    })
  }, 250)
  return () => { cancelled = true; clearTimeout(timer) }
}
