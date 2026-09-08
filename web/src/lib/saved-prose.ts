/** Saved structure is inert data: never parse its prose as HTML. */
export interface SavedBlock { tag?: string; attributes?: Record<string, unknown>; children?: SavedBlock[]; text?: { insert?: unknown; attributes?: Record<string, unknown> | null }[] }
export function savedBlocks(value: unknown): SavedBlock[] | null {
  return Array.isArray(value) ? value.filter((node): node is SavedBlock => !!node && typeof node === 'object') : null
}
export function savedText(value: unknown, titles: ReadonlyMap<string, string> = new Map(), missing = 'Missing section', diagramLabel?: string): string {
  const blocks = savedBlocks(value)
  if (!blocks) return ''
  return blocks.map(node => diagramLabel && node.tag === 'codeBlock' && ['mermaid', 'plantuml', 'puml', 'salt', 'd2'].includes(String(node.attributes?.language)) ? diagramLabel : node.tag === 'sectionReference' ? titles.get(String(node.attributes?.sectionId)) || savedText(node.children, titles, missing, diagramLabel) || missing : node.text ? node.text.map(run => typeof run.insert === 'string' ? run.insert : '').join('') : savedText(node.children, titles, missing, diagramLabel)).join(blocks.some(node => node.tag && !['sectionReference', 'hardBreak'].includes(node.tag)) ? '\n' : '')
}
/** Object-key ordering is not a document edit; arrays retain their order. */
export function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`
  if (value && typeof value === 'object') return `{${Object.entries(value).sort(([a], [b]) => a.localeCompare(b)).map(([key, child]) => `${JSON.stringify(key)}:${canonical(child)}`).join(',')}}`
  return JSON.stringify(value) ?? ''
}
export interface TextChange { text: string; changed: boolean }
export function wordChanges(before: string, after: string): [TextChange[], TextChange[]] {
  const a = before.match(/\s+|[^\s]+/g) ?? [], b = after.match(/\s+|[^\s]+/g) ?? []
  const sameA = new Set<number>(), sameB = new Set<number>()
  // Bound work for very long specification sections. The fallback highlights
  // the changed middle, preserving the common prefix and suffix exactly.
  if (a.length * b.length > 250_000) {
    let first = 0
    while (first < Math.min(a.length, b.length) && a[first] === b[first]) { sameA.add(first); sameB.add(first++) }
    let i = a.length - 1, j = b.length - 1
    while (i >= first && j >= first && a[i] === b[j]) { sameA.add(i--); sameB.add(j--) }
  } else {
    const grid = Array.from({ length: a.length + 1 }, () => new Uint32Array(b.length + 1))
    for (let i = a.length - 1; i >= 0; i--) for (let j = b.length - 1; j >= 0; j--) grid[i]![j] = a[i] === b[j] ? grid[i + 1]![j + 1]! + 1 : Math.max(grid[i + 1]![j]!, grid[i]![j + 1]!)
    let i = 0, j = 0
    while (i < a.length && j < b.length) {
      if (a[i] === b[j]) { sameA.add(i++); sameB.add(j++) }
      else if (grid[i + 1]![j]! >= grid[i]![j + 1]!) i++
      else j++
    }
  }
  return [a.map((text, i) => ({ text, changed: !sameA.has(i) })), b.map((text, i) => ({ text, changed: !sameB.has(i) }))]
}

/** Older snapshots still carry XML. Parse it as inert data, never insert markup. */
export function sectionBlocks(node: { prose_structure?: unknown; prose_xml?: string }): SavedBlock[] | null {
  const blocks = savedBlocks(node.prose_structure)
  if (blocks) return blocks
  if (!node.prose_xml || typeof DOMParser === 'undefined') return null
  const doc = new DOMParser().parseFromString(`<saved>${node.prose_xml}</saved>`, 'application/xml')
  if (doc.querySelector('parsererror')) return null
  const convert = (node: Node): SavedBlock => node.nodeType === 3 ? { text: [{ insert: node.textContent ?? '' }] } : {
    tag: (node as Element).tagName,
    attributes: Object.fromEntries(Array.from((node as Element).attributes ?? [], a => [a.name, a.value])),
    children: Array.from(node.childNodes, convert),
  }
  return Array.from(doc.documentElement.childNodes, convert)
}
