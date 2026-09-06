// Renders agent- and human-written markdown.
//
// The renderer stays imperative and this component is the only bridge to it:
// `replaceChildren` swaps in a freshly built DOM tree. That is deliberate and
// it is the security boundary — there is no `dangerouslySetInnerHTML` here or
// anywhere else (eslint.config.js makes that a hard error), so markup in a
// ticket body can never become markup in the page.
import { useEffect, useRef } from 'react'
import { createMermaidControls } from '../lib/mermaid-controls'
import { renderMarkdown } from '../lib/markdown'

export interface MarkdownProps {
  /** Markdown source. Anything non-string renders as empty. */
  text: string | null | undefined
  /** Extra class on the wrapper, alongside `md`. */
  variant?: string
  /** Applied to the mount point, not the rendered tree. */
  className?: string
}

export function Markdown({ text, variant, className }: MarkdownProps) {
  const host = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const node = host.current
    if (!node) return
    node.replaceChildren(renderMarkdown(text, variant))
    const cancel = Array.from(node.querySelectorAll('code[data-lang]')).flatMap((code) => {
      if (code.getAttribute('data-lang')?.toLowerCase() !== 'mermaid') return []
      const pre = code.parentElement
      if (!pre) return []
      const block = document.createElement('div')
      pre.before(block)
      block.append(pre)
      const controls = createMermaidControls(block, pre, code.textContent ?? '')
      return [() => controls.destroy()]
    })
    // Leave the host empty on unmount so a detached tree cannot be retained.
    return () => { cancel.forEach((stop) => stop()); node.replaceChildren() }
  }, [text, variant])

  return <div ref={host} className={className} />
}
