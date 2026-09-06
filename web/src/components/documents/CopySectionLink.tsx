import { useEffect, useState } from 'react'
import { Check, Link } from 'lucide-react'
import type { Locale } from '@/lib/i18n'

export function CopySectionLink({ href, locale }: { href: string; locale: Locale }) {
  const [status, setStatus] = useState<'idle' | 'copied' | 'manual'>('idle')
  const de = locale === 'de'
  useEffect(() => {
    setStatus('idle')
  }, [href])
  useEffect(() => {
    if (status !== 'copied') return
    const timer = setTimeout(() => setStatus('idle'), 2500)
    return () => clearTimeout(timer)
  }, [status])
  return <span className="inline-flex min-w-0 items-center gap-2">
    <button type="button" className="inline-flex items-center gap-1 text-muted-foreground hover:text-foreground"
      onMouseDown={event => event.preventDefault()}
      onClick={async () => {
        try { await navigator.clipboard.writeText(href); setStatus('copied') }
        catch { setStatus('manual') }
      }}>
      {status === 'copied' ? <Check className="size-3.5" aria-hidden="true" /> : <Link className="size-3.5" aria-hidden="true" />}
      {de ? 'Abschnittslink kopieren' : 'Copy section link'}
    </button>
    <span role="status">{status === 'copied' ? (de ? 'Kopiert' : 'Copied') : ''}</span>
    {status === 'manual' && <input readOnly value={href} className="min-w-0 w-48 rounded border bg-background p-1 text-xs"
      aria-label={de ? 'Link manuell kopieren' : 'Copy this link manually'} onFocus={event => event.target.select()} />}
  </span>
}
