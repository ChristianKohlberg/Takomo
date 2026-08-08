// The one imperative DOM helper the ported renderer needs.
//
// Everything else in these pages is React. This survives because the markdown
// renderer builds nodes directly — which is precisely what makes it impossible
// for agent-written text to inject markup — and rewriting it as JSX would trade
// that structural guarantee for a review convention.
export function el(tag: string, cls?: string | null, text?: string | null): HTMLElement {
  const n = document.createElement(tag)
  if (cls) n.className = cls
  if (text != null) n.textContent = text
  return n
}
