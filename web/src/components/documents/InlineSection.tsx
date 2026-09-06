import { useState } from 'react'
import type { Locale } from '@/lib/i18n'

/** A document line, always ready to type; Enter turns it into a map section. */
export function InlineSection({
  locale,
  maxLevel,
  onInsert,
}: {
  locale: Locale
  maxLevel: number
  onInsert: (level: 1 | 2 | 3, title: string) => boolean
}) {
  const [level, setLevel] = useState<1 | 2 | 3>(1)
  const [title, setTitle] = useState('')
  const [failed, setFailed] = useState(false)
  const de = locale === 'de'
  return (
    <form
      className="group my-2 flex min-w-0 flex-wrap items-center gap-2 text-sm"
      onSubmit={(event) => {
        event.preventDefault()
        const markdown = /^(#{1,3})\s+(.*)$/.exec(title)
        const nextLevel = (markdown ? markdown[1]!.length : level) as 1 | 2 | 3
        if (nextLevel > maxLevel || !onInsert(nextLevel, markdown ? markdown[2]! : title)) {
          setFailed(true)
          return
        }
        setTitle('')
        setFailed(false)
      }}
    >
      <select
        aria-label={de ? 'Überschriftenebene' : 'Heading level'}
        value={level}
        onChange={(event) => setLevel(Number(event.target.value) as 1 | 2 | 3)}
        className="text-muted-foreground rounded px-1 py-2 focus-visible:outline-2"
      >
        {[1, 2, 3].map((value) => (
          <option key={value} value={value} disabled={value > maxLevel}>H{value}</option>
        ))}
      </select>
      <input
        aria-label={de ? 'Abschnitt inline hinzufügen' : 'Add section inline'}
        placeholder={de ? 'Überschrift schreiben …' : 'Type a heading …'}
        className="min-w-0 flex-1 bg-transparent px-1 py-2 outline-none placeholder:text-muted-foreground focus-visible:ring-1 focus-visible:ring-ring"
        value={title}
        onChange={(event) => { setTitle(event.target.value); setFailed(false) }}
      />
      <button type="submit" className="text-muted-foreground hover:text-foreground rounded px-2 py-1" aria-label={de ? 'Abschnitt einfügen' : 'Insert section'}>↵</button>
      {failed && <p role="alert" className="text-destructive w-full text-xs">{de ? 'Abschnitt konnte nicht eingefügt werden. Prüfe die Ebene oder die maximale Abschnittsanzahl.' : 'Could not insert section. Check the heading level or section limit.'}</p>}
    </form>
  )
}
