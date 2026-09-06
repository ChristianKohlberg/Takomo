import { literalMatches } from './document-search'

/** CSS highlights leave contenteditable headings and their undo history intact. */
export function highlightDocumentHeadings(
  headings: readonly { element: HTMLElement; activeFrom?: number }[], query: string,
): () => void {
  if (typeof CSS === 'undefined' || !('highlights' in CSS) || typeof Highlight === 'undefined') return () => undefined
  const all: Range[] = []
  const active: Range[] = []
  for (const { element, activeFrom } of headings) {
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT)
    const nodes: { node: Text; start: number; end: number }[] = []
    let text = ''
    while (walker.nextNode()) {
      const node = walker.currentNode as Text
      nodes.push({ node, start: text.length, end: text.length + node.length })
      text += node.data
    }
    for (const match of literalMatches(text, query)) {
      const start = nodes.find(item => item.start <= match.from && item.end > match.from)
      const end = nodes.find(item => item.start < match.to && item.end >= match.to)
      if (!start || !end) continue
      const range = document.createRange()
      range.setStart(start.node, match.from - start.start)
      range.setEnd(end.node, match.to - end.start)
      all.push(range)
      if (match.from === activeFrom) active.push(range)
    }
  }
  CSS.highlights.set('document-search', new Highlight(...all))
  CSS.highlights.set('document-search-active', new Highlight(...active))
  return () => {
    CSS.highlights.delete('document-search')
    CSS.highlights.delete('document-search-active')
  }
}
