import { useEffect, useRef, type ReactNode } from 'react'
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
export function SavedProse({ node, nodes, access, missing }: { node: SavedSection; nodes: SavedSection[]; access: DiagramAccess; missing: string }) {
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
      case 'table': return <div key={key} className="max-w-full overflow-auto"><table className="w-full border-collapse"><tbody>{children}</tbody></table></div>
      case 'tableRow': return <tr key={key}>{children}</tr>
      case 'tableCell': return <td key={key} colSpan={span('colspan')} rowSpan={span('rowspan')} className="border p-2 align-top">{children}</td>
      case 'tableHeader': return <th key={key} colSpan={span('colspan')} rowSpan={span('rowspan')} className="border bg-muted p-2 text-left">{children}</th>
      case 'codeBlock': return diagramEngine(language) ? <SavedDiagram key={key} source={savedText(block.children)} language={language} access={access} /> : <pre key={key} className="max-w-full overflow-auto rounded bg-muted p-2 text-xs">{savedText(block.children)}</pre>
      default: return <span key={key}>{children}</span>
    }
  }
  return <div className="space-y-3 break-words text-sm">{blocks ? blocks.map(render) : <p className="whitespace-pre-wrap">{node.notes}</p>}</div>
}
