import { Extension } from '@tiptap/react'
import { Plugin, PluginKey } from '@tiptap/pm/state'

/** Keep structure and emphasis; let project typography own the appearance. */
export function cleanPastedHTML(html: string): string {
  const doc = new DOMParser().parseFromString(html, 'text/html')
  doc.querySelectorAll('script, style, meta, link, title').forEach(element => element.remove())
  for (const element of Array.from(doc.body.querySelectorAll<HTMLElement>('*'))) {
    const style = element.style
    const weight = style.fontWeight
    const emphasis: string[] = []
    if (weight === 'bold' || weight === 'bolder' || Number(weight) >= 600) emphasis.push('strong')
    if (style.fontStyle === 'italic' || style.fontStyle === 'oblique') emphasis.push('em')
    const decoration = `${style.textDecoration} ${style.textDecorationLine}`
    if (decoration.includes('underline')) emphasis.push('u')
    if (decoration.includes('line-through')) emphasis.push('s')
    // Google Docs wraps its entire export in <b style="font-weight:normal">.
    // Removing only the style would incorrectly make the whole paste bold.
    let target = element
    if ((element.tagName === 'B' || element.tagName === 'STRONG') &&
        (weight === 'normal' || (weight !== '' && Number(weight) < 600))) {
      target = doc.createElement('span')
      for (const attribute of Array.from(element.attributes)) target.setAttribute(attribute.name, attribute.value)
      target.append(...Array.from(element.childNodes))
      element.replaceWith(target)
    }
    for (const attribute of Array.from(target.attributes)) {
      if (['style', 'class', 'id', 'face', 'size', 'color', 'bgcolor', 'align', 'width', 'height',
        'data-id', 'data-colwidth'].includes(attribute.name)) {
        // Code language is semantic and selects the diagram renderer.
        if (attribute.name === 'class' && target.tagName === 'CODE') {
          const languages = attribute.value.split(/\s+/).filter(name => /^language-[\w-]+$/.test(name))
          if (languages.length) { target.setAttribute('class', languages.join(' ')); continue }
        }
        target.removeAttribute(attribute.name)
      }
    }
    // The editor schema remains responsible for accepting semantic content.
    if (emphasis.length) {
      // Wrap text, not table/list children: a <strong> around <tbody> or
      // <li> would create invalid HTML and lose structure when parsed again.
      const walker = doc.createTreeWalker(target, NodeFilter.SHOW_TEXT)
      const texts: Node[] = []
      while (walker.nextNode()) texts.push(walker.currentNode)
      for (const text of texts) {
        let child = text
        for (const tag of emphasis) {
          const wrapper = doc.createElement(tag)
          child.parentNode!.replaceChild(wrapper, child)
          wrapper.append(child)
          child = wrapper
        }
      }
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
