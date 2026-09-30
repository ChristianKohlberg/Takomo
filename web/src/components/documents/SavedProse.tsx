import { useEffect, useId, useRef, useState, type ReactNode } from 'react'
import { longTableCut, longTableLabels } from '@/lib/long-table'
import type { Locale } from '@/lib/i18n'
import '@/styles/document-long-table.css'
import { createDiagramControls } from '@/lib/diagram-controls'
import { diagramEngine, type DiagramAccess } from '@/lib/diagram'
import { sectionBlocks, savedText, type SavedBlock } from '@/lib/saved-prose'
import type { SavedSection } from '@/lib/spec-history'

function SavedDiagram({ source, language, access }: { source: string; language: string; access: DiagramAccess }) {
  const host = useRef<HTMLDivElement>(null)
  const { token, project } = access
  useEffect(() => {
    if (!host.current) return
    const pre = document.createElement('pre')
    pre.textContent = source
    host.current.replaceChildren(pre)
    const controls = createDiagramControls(host.current, pre, source, false, diagramEngine(language)!, { token, project })
    return () => controls.destroy()
  }, [source, language, token, project])
  return <div ref={host} />
}
/** A saved table; long ones start collapsed, as local view state (docs/documents.md "Tables"). */
function SavedTable({ rows, render, locale }: { rows: SavedBlock[]; render: (block: SavedBlock, key: number) => ReactNode; locale: Locale }) {
  const [expanded, setExpanded] = useState(false)
  const id = useId()
  const cut = longTableCut(rows.map(row => ({
    header: !!row.children?.length && row.children.every(cell => cell.tag === 'tableHeader'),
    rowspans: row.children?.map(cell => Math.max(1, Number(cell.attributes?.rowspan) || 1)) ?? [],
  })))
  const collapsed = !!cut && !expanded
  const labels = longTableLabels(locale)
  return <div className={cut ? 'document-long-table relative max-w-full overflow-auto' : 'max-w-full overflow-auto'} data-long-table={cut ? (collapsed ? 'collapsed' : 'expanded') : undefined}>
    <table id={`${id}-table`} className="w-full border-collapse" aria-describedby={collapsed ? `${id}-status` : undefined}>
      <tbody>{rows.map((row, index) => <tr key={index} className={collapsed && index >= cut.visibleRows ? 'document-long-table-hidden' : undefined}>{row.children?.map(render)}</tr>)}</tbody>
    </table>
    {cut && <div className="document-long-table-controls">
      <div className="document-long-table-fade" aria-hidden="true" />
      {collapsed && <span id={`${id}-status`} className="sr-only">{labels.status(cut.visibleBodyRows, cut.bodyRows)}</span>}
      <button type="button" className="document-long-table-toggle" aria-expanded={!collapsed} aria-controls={`${id}-table`} onClick={() => setExpanded(!expanded)}>
        {collapsed ? labels.showAll(cut.bodyRows) : labels.showLess}
      </button>
    </div>}
  </div>
}

export function SavedProse({ node, nodes, access, missing, locale = 'en' }: { node: SavedSection; nodes: SavedSection[]; access: DiagramAccess; missing: string; locale?: Locale }) {
  const blocks = sectionBlocks(node)
  function render(block: SavedBlock, key: number): ReactNode {
    if (block.text) return <span key={key}>{block.text.map((run, index) => {
      let text: ReactNode = typeof run.insert === 'string' ? run.insert : ''
      if (run.attributes?.bold) text = <strong>{text}</strong>
      if (run.attributes?.italic) text = <em>{text}</em>
      if (run.attributes?.strike) text = <s>{text}</s>
      if (run.attributes?.code) text = <code>{text}</code>
      const link = run.attributes?.link as { href?: unknown } | undefined
      if (typeof link?.href === 'string' && /^(https?:|mailto:|\/(?!\/)|#)/i.test(link.href)) text = <a className="text-primary underline" href={link.href} rel="noopener noreferrer">{text}</a>
      return <span key={index}>{text}</span>
    })}</span>
    const children = block.children?.map(render)
    const language = String(block.attributes?.language ?? '')
    const span = (name: string) => Math.min(100, Math.max(1, Number(block.attributes?.[name]) || 1))
    switch (block.tag) {
      case 'sectionReference': return <span key={key} className="text-primary underline">{nodes.find(n => n.id === block.attributes?.sectionId)?.title || (savedText(block.children) ? `${savedText(block.children)} (${missing})` : missing)}</span>
      case 'bold': return <strong key={key}>{children}</strong>
      case 'italic': return <em key={key}>{children}</em>
      case 'paragraph': return <p key={key} className="whitespace-pre-wrap">{children}</p>
      case 'heading': return <p key={key} className="font-semibold">{children}</p>
      case 'bulletList': return <ul key={key} className="list-disc pl-5">{children}</ul>
      case 'orderedList': return <ol key={key} className="list-decimal pl-5">{children}</ol>
      case 'listItem': return <li key={key}>{children}</li>
      case 'blockquote': return <blockquote key={key} className="border-l-2 pl-3">{children}</blockquote>
      case 'hardBreak': return <br key={key} />
      case 'horizontalRule': return <hr key={key} />
      case 'collapsibleBlock': return <details key={key} className="rounded border p-2">{children}</details>
      case 'collapsibleSummary': return <summary key={key} className="cursor-pointer font-medium">{children}</summary>
      case 'collapsibleContent': return <div key={key} className="min-w-0 space-y-2">{children}</div>
      case 'table': return <SavedTable key={key} rows={(block.children ?? []).filter(row => row.tag === 'tableRow')} render={render} locale={locale} />
      case 'tableRow': return <tr key={key}>{children}</tr>
      case 'tableCell': return <td key={key} colSpan={span('colspan')} rowSpan={span('rowspan')} className="border p-2 align-top">{children}</td>
      case 'tableHeader': return <th key={key} colSpan={span('colspan')} rowSpan={span('rowspan')} className="border bg-muted p-2 text-left">{children}</th>
      case 'codeBlock': return diagramEngine(language) ? <SavedDiagram key={key} source={savedText(block.children)} language={language} access={access} /> : <pre key={key} className="max-w-full overflow-auto rounded bg-muted p-2 text-xs">{savedText(block.children)}</pre>
      default: return <span key={key}>{children}</span>
    }
  }
  return <div className="space-y-3 break-words text-sm">{blocks ? blocks.map(render) : <p className="whitespace-pre-wrap">{node.notes}</p>}</div>
}
