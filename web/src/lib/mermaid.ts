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
  image.alt = 'Mermaid diagram'
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
  host.textContent = 'Rendering diagram…'
  if (source.length > 50_000) {
    host.textContent = 'Diagram is too large to render. The source is available below.'
    return () => { cancelled = true }
  }
  const timer = setTimeout(() => {
    void renderImage(source).then((image) => {
      if (!cancelled) host.replaceChildren(image)
    }).catch(() => {
      if (!cancelled) host.textContent = 'Unable to render Mermaid diagram. Check the source below.'
    })
  }, 250)
  return () => { cancelled = true; clearTimeout(timer) }
}
