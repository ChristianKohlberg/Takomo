import * as Y from 'yjs'

export const LOCAL_EDIT = 'takomo:local-edit'

/** Change only the edited span, retaining the identities of all other characters. */
export function spliceText(text: Y.Text, value: string) {
  const old = text.toString()
  if (old === value) return
  let start = 0
  while (start < old.length && start < value.length && old[start] === value[start]) start++
  // Never split a surrogate pair.
  if (start > 0 && /[\uD800-\uDBFF]/.test(old[start - 1]!)) start--
  let end = 0
  while (end < old.length - start && end < value.length - start &&
    old[old.length - end - 1] === value[value.length - end - 1]) end++
  if (end > 0 && /[\uDC00-\uDFFF]/.test(old[old.length - end]!)) end--
  text.doc!.transact(() => {
    const count = old.length - start - end
    if (count) text.delete(start, count)
    const inserted = value.slice(start, value.length - end)
    if (inserted) text.insert(start, inserted)
  }, LOCAL_EDIT)
}
