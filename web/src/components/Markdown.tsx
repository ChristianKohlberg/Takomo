import { DiagramContext, diagramEngine, type DiagramAccess } from '../lib/diagram'
// Renders agent- and human-written markdown.
//
// The renderer stays imperative and this component is the only bridge to it:
// `replaceChildren` swaps in a freshly built DOM tree. That is deliberate and
// it is the security boundary — there is no `dangerouslySetInnerHTML` here or
// anywhere else (eslint.config.js makes that a hard error), so markup in a
// ticket body can never become markup in the page.
import { useContext, useEffect, useRef } from 'react'
import { createDiagramControls } from '../lib/diagram-controls'
import { renderMarkdown } from '../lib/markdown'

export interface MarkdownProps {
  /** Markdown source. Anything non-string renders as empty. */
  diagramAccess?: DiagramAccess
  text: string | null | undefined
  /** Extra class on the wrapper, alongside `md`. */
  variant?: string
  /** Applied to the mount point, not the rendered tree. */
  className?: string
}

export function Markdown({ text, variant, className, diagramAccess }: MarkdownProps) {
  const inherited = useContext(DiagramContext)
  const token = (diagramAccess ?? inherited)?.token
  const project = (diagramAccess ?? inherited)?.project
  const host = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const node = host.current
    if (!node) return
    node.replaceChildren(renderMarkdown(text, variant))
    const cancel = Array.from(node.querySelectorAll('code[data-lang]')).flatMap((code) => {
      const engine = diagramEngine(code.getAttribute('data-lang'))
      if (!engine) return []
      const pre = code.parentElement
      if (!pre) return []
      const block = document.createElement('div')
      pre.before(block)
      block.append(pre)
      const controls = createDiagramControls(block, pre, code.textContent ?? '', false, engine, token && project ? { token, project } : null)
      return [() => controls.destroy()]
    })
    // Leave the host empty on unmount so a detached tree cannot be retained.
    return () => { cancel.forEach((stop) => stop()); node.replaceChildren() }
  }, [text, variant, token, project])

  return <div ref={host} className={className} />
}
