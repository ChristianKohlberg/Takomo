import { useSyncExternalStore, useCallback } from 'react'
import type * as Y from 'yjs'
import { nodesMap, proseTextOf } from '@/lib/mindmap-crdt'
import type { Locale } from '@/lib/i18n'

/** Only folded sections subscribe to their summary; prose remains in the CRDT. */
export function SectionSummary({ ydoc, sectionId, childrenCount, locale }: {
  ydoc: Y.Doc; sectionId: string; childrenCount: number; locale: Locale
}) {
  const subscribe = useCallback((changed: () => void) => {
    const node = nodesMap(ydoc).get(sectionId)
    node?.observeDeep(changed)
    return () => node?.unobserveDeep(changed)
  }, [ydoc, sectionId])
  const read = useCallback(() => proseTextOf(ydoc, sectionId).replace(/\s+/g, ' ').trim(), [ydoc, sectionId])
  const text = useSyncExternalStore(subscribe, read)
  return <div className="text-muted-foreground mb-3 text-sm">
    {text && <p className="line-clamp-2 break-words">{text.length > 220 ? `${text.slice(0, 220)}…` : text}</p>}
    {childrenCount > 0 && <p className="mt-1 text-xs">{locale === 'de' ? `${childrenCount} ${childrenCount === 1 ? 'Unterabschnitt' : 'Unterabschnitte'}` : `${childrenCount} ${childrenCount === 1 ? 'subsection' : 'subsections'}`}</p>}
    {!text && !childrenCount && <p>{locale === 'de' ? 'Noch kein Inhalt.' : 'No content yet.'}</p>}
  </div>
}
