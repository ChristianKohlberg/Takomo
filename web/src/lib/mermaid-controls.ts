import { defineStrings, detectLocale } from './i18n'
import { mountMermaid } from './mermaid'

const strings = defineStrings({
  en: { diagram: 'Diagram', code: 'Code', size: 'Diagram size', compact: 'Compact', comfortable: 'Comfortable', original: 'Original', expand: 'Expand diagram', close: 'Close', fit: 'Fit', zoomIn: 'Zoom in', zoomOut: 'Zoom out', title: 'Mermaid diagram' },
  de: { diagram: 'Diagramm', code: 'Code', size: 'Diagrammgröße', compact: 'Kompakt', comfortable: 'Komfortabel', original: 'Original', expand: 'Diagramm vergrößern', close: 'Schließen', fit: 'Einpassen', zoomIn: 'Vergrößern', zoomOut: 'Verkleinern', title: 'Mermaid-Diagramm' },
})
type View = 'diagram' | 'code'
type Size = 'compact' | 'comfortable' | 'original'
const preferenceKey = 'takomo.mermaid.preferences.v1'
const sizes: Size[] = ['compact', 'comfortable', 'original']
function read(key: string): string | null {
  try { return localStorage.getItem(key) } catch { return null }
}

/** View state is device-local; source remains owned by Markdown/ProseMirror. */
export function createMermaidControls(root: HTMLElement, pre: HTMLElement, source: string, startInCode = false) {
  const t = strings[detectLocale(read('takomo.lang'))]
  let view: View = 'diagram'
  let size: Size = 'compact'
  try {
    const saved: unknown = JSON.parse(read(preferenceKey) ?? '{}')
    if (saved && typeof saved === 'object') {
      if ('view' in saved && saved.view === 'code') view = 'code'
      if ('size' in saved && sizes.includes(saved.size as Size)) size = saved.size as Size
    }
  } catch { /* Ignore malformed or unavailable browser storage. */ }
  if (startInCode) view = 'code'
  const controls = document.createElement('div')
  controls.className = 'mermaid-controls'
  controls.contentEditable = 'false'
  const preview = document.createElement('div')
  preview.className = 'mermaid-preview'
  preview.contentEditable = 'false'
  const toolbar = document.createElement('div')
  toolbar.className = 'mermaid-toolbar'
  const button = (label: string, action: () => void) => {
    const node = document.createElement('button')
    node.type = 'button'
    node.textContent = label
    node.addEventListener('click', action)
    return node
  }
  const persist = () => {
    try { localStorage.setItem(preferenceKey, JSON.stringify({ view, size })) } catch { /* Controls still work without storage. */ }
  }
  let cancel: (() => void) | undefined
  let closeOverlay: (() => void) | undefined
  let renderingSource: string | undefined
  const paint = () => {
    pre.hidden = view !== 'code'
    preview.hidden = view !== 'diagram'
    sizeSelect.hidden = expand.hidden = view !== 'diagram'
    diagramButton.setAttribute('aria-pressed', String(view === 'diagram'))
    codeButton.setAttribute('aria-pressed', String(view === 'code'))
    root.dataset.mermaidSize = size
    if (view === 'diagram' && renderingSource !== source) {
      cancel?.()
      renderingSource = source
      cancel = mountMermaid(preview, source)
    }
  }
  const show = (next: View, remember = true) => {
    view = next
    paint()
    if (remember) persist()
  }
  const diagramButton = button(t.diagram, () => show('diagram'))
  const codeButton = button(t.code, () => show('code'))
  const sizeSelect = document.createElement('select')
  sizeSelect.setAttribute('aria-label', t.size)
  for (const choice of sizes) {
    const option = document.createElement('option')
    option.value = choice
    option.textContent = t[choice]
    sizeSelect.append(option)
  }
  sizeSelect.value = size
  sizeSelect.addEventListener('change', () => { size = sizeSelect.value as Size; paint(); persist() })
  const expand = button(t.expand, () => {
    if (closeOverlay) return
    const dialog = document.createElement('dialog')
    dialog.className = 'mermaid-dialog'
    dialog.setAttribute('aria-label', t.title)
    const tools = document.createElement('div')
    tools.className = 'mermaid-toolbar'
    const canvas = document.createElement('div')
    canvas.className = 'mermaid-dialog-canvas'
    let zoom: number | undefined
    const zoomLabel = document.createElement('output')
    zoomLabel.setAttribute('aria-live', 'polite')
    const update = () => {
      canvas.replaceChildren(...Array.from(preview.childNodes, node => node.cloneNode(true)))
      const image = canvas.querySelector('img')
      zoomLabel.textContent = zoom === undefined ? t.fit : `${Math.round(zoom * 100)}%`
      canvas.dataset.fit = String(zoom === undefined)
      if (image && zoom !== undefined) {
        image.style.width = `${(Number(image.getAttribute('width')) || image.naturalWidth || 800) * zoom}px`
        image.style.maxWidth = 'none'
        image.style.maxHeight = 'none'
        image.style.height = 'auto'
      }
    }
    const observer = new MutationObserver(update)
    observer.observe(preview, { childList: true, subtree: true })
    const zoomTo = (value?: number) => { zoom = value; update() }
    const close = () => {
      observer.disconnect()
      dialog.remove()
      closeOverlay = undefined
      if (expand.isConnected) expand.focus({ preventScroll: true })
    }
    const closeButton = button(t.close, close)
    tools.append(button(t.fit, () => zoomTo()), button('100%', () => zoomTo(1)), button('−', () => zoomTo(Math.max(0.1, (zoom ?? fittedScale()) - 0.25))), zoomLabel, button('+', () => zoomTo(Math.min(4, (zoom ?? fittedScale()) + 0.25))), closeButton)
    tools.children[2]!.setAttribute('aria-label', t.zoomOut)
    tools.children[4]!.setAttribute('aria-label', t.zoomIn)
    function fittedScale() {
      const image = preview.querySelector('img')
      return image ? Math.min(1, canvas.clientWidth / (Number(image.getAttribute('width')) || 800), canvas.clientHeight / (Number(image.getAttribute('height')) || 600)) : 1
    }
    dialog.append(tools, canvas)
    document.body.append(dialog)
    closeOverlay = close
    dialog.addEventListener('cancel', event => { event.preventDefault(); close() })
    dialog.addEventListener('close', close)
    dialog.showModal()
    update()
    closeButton.focus()
  })
  toolbar.append(sizeSelect, expand, diagramButton, codeButton)
  controls.append(toolbar, preview)
  root.classList.add('mermaid-block')
  root.prepend(controls)
  paint()
  return {
    controls,
    showCode: () => show('code', false),
    update(next: string) { source = next; paint() },
    destroy() {
      cancel?.()
      closeOverlay?.()
      controls.remove()
      root.classList.remove('mermaid-block')
      delete root.dataset.mermaidSize
      pre.hidden = false
    },
  }
}
