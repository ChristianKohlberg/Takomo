import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createDiagramControls } from './diagram-controls'
const { mount, cancel } = vi.hoisted(() => ({ mount: vi.fn(), cancel: vi.fn() }))
vi.mock('./diagram', async (original) => ({ ...await original<typeof import('./diagram')>(), mountDiagram: mount }))
const key = 'takomo.mermaid.preferences.v1'
function fixture(source = 'flowchart TD\nA --> B', empty = false, id?: string) {
  const root = document.createElement('div')
  const pre = document.createElement('pre')
  pre.textContent = source
  if (id) root.dataset.id = id
  root.append(pre)
  document.body.append(root)
  return { root, pre, controls: createDiagramControls(root, pre, source, empty) }
}
function click(root: ParentNode, label: string) {
  const button = Array.from(root.querySelectorAll('button')).find(node => node.textContent === label)!
  button.click()
  return button
}
beforeEach(() => {
  localStorage.clear()
  mount.mockImplementation((host: HTMLElement) => {
    const image = document.createElement('img')
    image.width = 1000
    image.height = 600
    host.replaceChildren(image)
    return cancel
  })
  Object.defineProperty(HTMLDialogElement.prototype, 'showModal', { configurable: true, value: function (this: HTMLDialogElement) { this.setAttribute('open', '') } })
})
afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); vi.clearAllMocks() })

describe('personal Mermaid controls', () => {
  it('defaults to compact diagram and keeps source intact across remembered view and size changes', () => {
    const { root, pre, controls } = fixture()
    expect(pre.hidden).toBe(true)
    expect(root.dataset.mermaidSize).toBe('compact')
    const text = pre.textContent
    click(root, 'Code')
    expect(pre.hidden).toBe(false)
    expect(root.querySelector('.mermaid-preview')?.hasAttribute('hidden')).toBe(true)
    click(root, 'View')
    const select = root.querySelector('select')!
    select.value = 'comfortable'
    select.dispatchEvent(new Event('change'))
    click(root, 'Code')
    expect(pre.textContent).toBe(text)
    controls.destroy()
    const next = fixture()
    expect(next.pre.hidden).toBe(false)
    expect(next.root.dataset.mermaidSize).toBe('comfortable')
    next.controls.destroy()
  })
  it('collapses locally, retains its summary and source, and defers changed previews until revealed', () => {
    const { root, pre, controls } = fixture('%% Orders and their owners\nflowchart TD\nA --> B', false, 'blk-orders')
    const original = pre.textContent
    const toggle = click(root, 'Collapse')
    expect(toggle.getAttribute('aria-expanded')).toBe('false')
    expect(pre.hidden).toBe(true)
    expect(root.querySelector('.mermaid-preview')?.hasAttribute('hidden')).toBe(true)
    expect(root.querySelector('.diagram-description')?.textContent).toContain('Orders and their owners')
    mount.mockClear()
    controls.update('%% Updated relationships\nflowchart TD\nB --> C')
    expect(mount).not.toHaveBeenCalled()
    expect(root.querySelector('.diagram-description')?.textContent).toContain('Updated relationships')
    expect(pre.textContent).toBe(original)
    controls.destroy()
    const same = fixture('%% Updated relationships\nflowchart TD\nB --> C', false, 'blk-orders')
    expect(mount).not.toHaveBeenCalled()
    click(same.root, 'Show diagram')
    expect(mount).toHaveBeenCalledTimes(1)
    same.controls.destroy()
    const other = fixture('flowchart TD\nA --> B', false, 'blk-other')
    expect(other.root.querySelector('.mermaid-preview')?.hasAttribute('hidden')).toBe(false)
    click(other.root, 'Collapse')
    other.controls.showCode()
    expect(other.pre.hidden).toBe(false)
    other.controls.destroy()
  })
  it('remembers a newly inserted block after its stable ID is assigned', () => {
    const first = fixture()
    first.root.dataset.id = 'blk-new'
    click(first.root, 'Collapse')
    first.controls.destroy()
    mount.mockClear()
    const restored = fixture('flowchart TD\nA --> B', false, 'blk-new')
    expect(mount).not.toHaveBeenCalled()
    expect(restored.root.querySelector('button')?.getAttribute('aria-expanded')).toBe('false')
    restored.controls.destroy()
  })
  it('shows DBML summaries as literal text and falls back to a line count', () => {
    const root = document.createElement('div')
    const pre = document.createElement('pre')
    root.append(pre)
    const controls = createDiagramControls(root, pre, '// <img src=x onerror=alert(1)>\nTable users {\n id integer [pk]\n}', false, 'dbml')
    click(root, 'Collapse')
    expect(root.querySelector('.diagram-description')?.textContent).toBe('DBML<img src=x onerror=alert(1)>')
    expect(root.querySelector('.diagram-description img')).toBeNull()
    controls.update('Table users {\n id integer [pk]\n}')
    expect(root.querySelector('.diagram-description')?.textContent).toBe('DBML3 lines')
    controls.destroy()
  })
  it('tolerates blocked or malformed preference storage and opens an empty writer in Code', () => {
    localStorage.setItem(key, 'invalid')
    const malformed = fixture()
    expect(malformed.pre.hidden).toBe(true)
    malformed.controls.destroy()
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('blocked') })
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('blocked') })
    const { root, pre, controls } = fixture('', true)
    expect(pre.hidden).toBe(false)
    click(root, 'View')
    expect(pre.hidden).toBe(true)
    controls.destroy()
  })
  it('cleans up old renders, zooms using intrinsic dimensions, and restores overlay focus on Escape', () => {
    const { root, controls } = fixture()
    controls.update('flowchart TD\nB --> C')
    expect(cancel).toHaveBeenCalledTimes(1)
    const expand = click(root, 'Expand diagram')
    const dialog = document.querySelector('dialog')!
    expect(dialog.open).toBe(true)
    expect(document.activeElement?.textContent).toBe('Close')
    click(dialog, '100%')
    expect(dialog.querySelector('img')?.style.width).toBe('1000px')
    click(dialog, '+')
    expect(dialog.querySelector('img')?.style.width).toBe('1250px')
    click(dialog, 'Fit')
    expect(dialog.querySelector('[data-fit=true]')).not.toBeNull()
    dialog.dispatchEvent(new Event('cancel', { cancelable: true }))
    expect(document.querySelector('dialog')).toBeNull()
    expect(document.activeElement).toBe(expand)
    click(root, 'Expand diagram')
    controls.destroy()
    expect(document.querySelector('dialog')).toBeNull()
    expect(cancel).toHaveBeenCalledTimes(2)
  })
})
