// The live editor's table view: Tiptap's TableView plus the long-table collapse.
//
// Plugged in through `TableKit.configure({ table: { View: LongTableView } })`,
// which Tiptap uses both for the resizable (editable) path — prosemirror-tables'
// column-resizing plugin constructs it — and for the plain read-only path. So
// writers and readers get the same behaviour from one class.
//
// Everything here is local DOM state owned by the node view, exactly like the
// collapsible block's open/closed state: no transaction, no node attribute, no
// Yjs write, no undo entry. ProseMirror reuses a node view across updates —
// including y-sync's whole-document replace when a remote edit arrives — so an
// expanded table stays expanded while somebody else types.
//
// Rows are hidden with a per-table `<style>` rule rather than attributes on the
// `<tr>`s: the rows belong to ProseMirror, and touching their attributes would
// be read back as a DOM change to the document. The style element, like the
// controls, lives in the wrapper outside `contentDOM`, where TableView already
// tells ProseMirror to ignore mutations.
import { TableView } from '@tiptap/extension-table'
import { Extension, type Editor } from '@tiptap/react'
import { Plugin, PluginKey, type EditorState } from '@tiptap/pm/state'
import type { Node as PMNode } from '@tiptap/pm/model'
import type { EditorView } from '@tiptap/pm/view'
import { longTableCut, longTableLabels, type LongTableCut, type LongTableLabels } from './long-table'

const tablesOf = new WeakMap<EditorView, Set<LongTableView>>()
const labelsOf = new WeakMap<EditorView, () => LongTableLabels>()
let sequence = 0

/** Rows as `longTableCut` wants them, read from a ProseMirror table node. */
export function tableRows(node: PMNode): { header: boolean; rowspans: number[] }[] {
  const rows: { header: boolean; rowspans: number[] }[] = []
  node.forEach(row => {
    let header = row.childCount > 0
    const rowspans: number[] = []
    row.forEach(cell => {
      if (cell.type.name !== 'tableHeader') header = false
      rowspans.push(Number(cell.attrs.rowspan) || 1)
    })
    rows.push({ header, rowspans })
  })
  return rows
}

export class LongTableView extends TableView {
  private readonly view: EditorView | undefined
  private readonly controls: HTMLDivElement
  private readonly toggle: HTMLButtonElement
  private readonly status: HTMLSpanElement
  private readonly rule: HTMLStyleElement
  private cut: LongTableCut | null = null
  /** Local reading state. A freshly mounted table starts collapsed. */
  expanded = false
  private proposed = false

  constructor(node: PMNode, cellMinWidth: number, view?: EditorView, HTMLAttributes?: Record<string, unknown>) {
    super(node, cellMinWidth, view, HTMLAttributes)
    this.view = view
    const id = `long-table-${++sequence}`
    this.dom.id = id
    this.table.id = `${id}-table`
    this.rule = document.createElement('style')
    this.controls = document.createElement('div')
    this.controls.className = 'document-long-table-controls'
    this.controls.contentEditable = 'false'
    const fade = document.createElement('div')
    fade.className = 'document-long-table-fade'
    fade.setAttribute('aria-hidden', 'true')
    this.status = document.createElement('span')
    this.status.className = 'sr-only'
    this.status.id = `${id}-status`
    this.toggle = document.createElement('button')
    this.toggle.type = 'button'
    this.toggle.className = 'document-long-table-toggle'
    this.toggle.setAttribute('aria-controls', this.table.id)
    this.toggle.addEventListener('click', () => this.setExpanded(!this.expanded))
    this.controls.append(fade, this.status, this.toggle)
    this.dom.append(this.rule, this.controls)
    this.dom.addEventListener('reveal-long-table', event => {
      const target = (event as CustomEvent<Element | null>).detail
      if (!target || this.hiddenRowOf(target)) this.setExpanded(true)
    })
    if (view) {
      let set = tablesOf.get(view)
      if (!set) tablesOf.set(view, set = new Set())
      set.add(this)
    }
    this.measure()
  }

  override update(node: PMNode): boolean {
    if (!super.update(node)) return false
    this.measure()
    return true
  }

  stopEvent(event: Event): boolean {
    return this.controls.contains(event.target as globalThis.Node)
  }

  destroy(): void {
    if (this.view) tablesOf.get(this.view)?.delete(this)
  }

  setExpanded(expanded: boolean): void {
    if (this.expanded === expanded) return
    this.expanded = expanded
    this.render()
  }

  /** Is `element` inside a row that the current cut hides? */
  private hiddenRowOf(element: Element): boolean {
    if (!this.cut) return false
    for (let row: Element | null = element; row; row = row.parentElement) {
      if (row.parentElement === this.contentDOM) return Array.prototype.indexOf.call(this.contentDOM.children, row) >= this.cut.visibleRows
    }
    return false
  }

  private measure(): void {
    this.cut = longTableCut(tableRows(this.node))
    this.render()
  }

  render(): void {
    const cut = this.cut
    const collapsed = !!cut && !this.expanded
    this.dom.classList.toggle('document-long-table', !!cut)
    if (cut) this.dom.dataset.longTable = collapsed ? 'collapsed' : 'expanded'
    else delete this.dom.dataset.longTable
    const css = collapsed ? `#${this.dom.id}[data-long-table="collapsed"] > table > tbody > tr:nth-child(n+${cut.visibleRows + 1}) { display: none; }` : ''
    if (this.rule.textContent !== css) this.rule.textContent = css
    this.controls.hidden = !cut
    if (!cut) { this.table.removeAttribute('aria-describedby'); return }
    const labels = (this.view && labelsOf.get(this.view)?.()) || longTableLabels('en')
    this.toggle.setAttribute('aria-expanded', String(!collapsed))
    const text = collapsed ? labels.showAll(cut.bodyRows) : labels.showLess
    if (this.toggle.textContent !== text) this.toggle.textContent = text
    const status = collapsed ? labels.status(cut.visibleBodyRows, cut.bodyRows) : ''
    if (this.status.textContent !== status) this.status.textContent = status
    if (collapsed) this.table.setAttribute('aria-describedby', this.status.id)
    else this.table.removeAttribute('aria-describedby')
  }

  /**
   * Expand when something points into the hidden rows: the selection moving
   * there (caret, cell selection, comment or passage links), or a pending
   * proposal's highlight arriving on the table or a block around it.
   */
  sync(view: EditorView, previous: EditorState | undefined): void {
    const proposed = !!this.dom.closest('.takomo-proposed')
    const proposalArrived = proposed && !this.proposed
    this.proposed = proposed
    if (!this.cut || this.expanded) return
    if (proposalArrived) { this.setExpanded(true); return }
    const { selection } = view.state
    if (previous && previous.selection.eq(selection)) return
    let start: number
    try { start = view.posAtDOM(this.contentDOM, 0) } catch { return }
    let hiddenFrom = start
    for (let index = 0; index < this.cut.visibleRows; index++) hiddenFrom += this.node.child(index).nodeSize
    const end = start + this.node.content.size
    if (selection.to > hiddenFrom && selection.from < end) this.setExpanded(true)
  }
}

export function refreshLongTableLabels(editor: Editor): void {
  tablesOf.get(editor.view)?.forEach(table => table.render())
}

/** Supplies labels and runs the expand-on-selection/proposal checks after every update. */
export const LongTables = Extension.create<{ labels: () => LongTableLabels }>({
  name: 'longTables',
  addOptions: () => ({ labels: () => longTableLabels('en') }),
  addProseMirrorPlugins() {
    const labels = () => this.options.labels()
    return [new Plugin({
      key: new PluginKey('longTables'),
      view(view) {
        labelsOf.set(view, labels)
        const all = () => tablesOf.get(view) ?? new Set<LongTableView>()
        all().forEach(table => { table.render(); table.sync(view, undefined) })
        return {
          update(current, previous) { all().forEach(table => table.sync(current, previous)) },
        }
      },
    })]
  },
})
