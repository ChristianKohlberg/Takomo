import { Extension } from '@tiptap/react'
import { Plugin, PluginKey } from '@tiptap/pm/state'

type Emphasis = 'strong' | 'em' | 'u' | 's'
const EMPHASIS: Emphasis[] = ['strong', 'em', 'u', 's']
const SEMANTIC: Record<string, Emphasis> = { B: 'strong', STRONG: 'strong', I: 'em', EM: 'em', U: 'u', S: 's', STRIKE: 's', DEL: 's' }
const STRIPPED = ['style', 'class', 'id', 'face', 'size', 'color', 'bgcolor', 'align', 'width', 'height', 'data-id', 'data-colwidth']

// Google Docs wraps its entire export in <b style="font-weight:normal">.
// Removing only the style would incorrectly make the whole paste bold.
function declaredEmphasis(element: HTMLElement): Partial<Record<Emphasis, boolean>> {
  const style = element.style
  const declared: Partial<Record<Emphasis, boolean>> = {}
  const weight = style.fontWeight
  if (weight === 'bold' || weight === 'bolder' || Number(weight) >= 600) declared.strong = true
  else if (weight === 'normal' || weight === 'lighter' || (weight !== '' && Number(weight) < 600)) declared.strong = false
  if (style.fontStyle === 'italic' || style.fontStyle === 'oblique') declared.em = true
  else if (style.fontStyle === 'normal') declared.em = false
  const decoration = `${style.textDecoration} ${style.textDecorationLine}`
  if (decoration.includes('underline')) declared.u = true
  if (decoration.includes('line-through')) declared.s = true
  if (decoration.includes('none')) {
    declared.u ??= false
    declared.s ??= false
  }
  const semantic = SEMANTIC[element.tagName]
  if (semantic) declared[semantic] ??= true
  return declared
}

/** Keep structure and emphasis; let project typography own the appearance. */
export function cleanPastedHTML(html: string): string {
  const doc = new DOMParser().parseFromString(html, 'text/html')
  doc.querySelectorAll('script, style, meta, link, title').forEach(element => element.remove())
  const elements = Array.from(doc.body.querySelectorAll<HTMLElement>('*'))
  const declarations = new Map(elements.map(element => [element, declaredEmphasis(element)]))
  const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_TEXT)
  const texts: Text[] = []
  while (walker.nextNode()) texts.push(walker.currentNode as Text)
  const ancestors = (text: Text): HTMLElement[] => {
    const chain: HTMLElement[] = []
    for (let node = text.parentElement; node && node !== doc.body; node = node.parentElement) chain.push(node)
    return chain
  }
  const effective = new Map<Text, Partial<Record<Emphasis, boolean>>>()
  const demoted = new Set<HTMLElement>()
  for (const text of texts) {
    const chain = ancestors(text)
    const decided: Partial<Record<Emphasis, boolean>> = {}
    for (const kind of EMPHASIS) {
      const decider = chain.find(element => declarations.get(element)?.[kind] !== undefined)
      if (!decider) continue
      const value = declarations.get(decider)![kind]!
      decided[kind] = value
      if (!value) for (const element of chain) if (SEMANTIC[element.tagName] === kind) demoted.add(element)
    }
    effective.set(text, decided)
  }
  for (const element of elements) {
    for (const attribute of Array.from(element.attributes)) {
      if (!STRIPPED.includes(attribute.name)) continue
      // Code language is semantic and selects the diagram renderer.
      if (attribute.name === 'class' && element.tagName === 'CODE') {
        const languages = attribute.value.split(/\s+/).filter(name => /^language-[\w-]+$/.test(name))
        if (languages.length) { element.setAttribute('class', languages.join(' ')); continue }
      }
      element.removeAttribute(attribute.name)
    }
  }
  for (const element of demoted) {
    const span = doc.createElement('span')
    for (const attribute of Array.from(element.attributes)) span.setAttribute(attribute.name, attribute.value)
    span.append(...Array.from(element.childNodes))
    element.replaceWith(span)
  }
  // The editor schema remains responsible for accepting semantic content.
  // Wrap text, not table/list children: a <strong> around <tbody> or <li>
  // would create invalid HTML and lose structure when parsed again.
  for (const text of texts) {
    const decided = effective.get(text)!
    const chain = ancestors(text)
    let child: Node = text
    for (const kind of EMPHASIS) {
      if (!decided[kind] || chain.some(element => SEMANTIC[element.tagName] === kind)) continue
      const wrapper = doc.createElement(kind)
      child.parentNode!.replaceChild(wrapper, child)
      wrapper.append(child)
      child = wrapper
    }
  }
  return doc.body.innerHTML
}

export const CleanPaste = Extension.create({
  name: 'cleanPaste',
  addProseMirrorPlugins() {
    return [new Plugin({
      key: new PluginKey('cleanPaste'),
      props: { transformPastedHTML: cleanPastedHTML },
    })]
  },
})
