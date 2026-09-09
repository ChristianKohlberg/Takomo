import { createContext, useCallback, useContext, useEffect, useId, useRef, useState, type ReactNode } from 'react'
import { useBlocker } from 'react-router'
import type { Locale } from '@/lib/i18n'
import { ConfirmDialog } from './ConfirmDialog'
const DraftContext = createContext<((id: string, dirty: boolean) => void) | null>(null)
/** Existing editors also render outside settings; no provider means no route guard. */
export function useSettingsDraft(dirty: boolean) {
  const register = useContext(DraftContext)
  const id = useId()
  useEffect(() => { register?.(id, dirty); return () => register?.(id, false) }, [register, id, dirty])
}
export function SettingsDrafts({ children, lang }: { children: ReactNode; lang: Locale }) {
  const [drafts, setDrafts] = useState<Record<string, boolean>>({})
  const register = useCallback((id: string, dirty: boolean) => setDrafts(current => {
    if (!!current[id] === dirty) return current
    const next = { ...current }
    if (dirty) next[id] = true
    else delete next[id]
    return next
  }), [])
  const proceeding = useRef(false)
  const dirty = Object.values(drafts).some(Boolean)
  const blocker = useBlocker(({ currentLocation, nextLocation }) => dirty && (currentLocation.pathname !== nextLocation.pathname || currentLocation.search !== nextLocation.search))
  useEffect(() => { if (blocker.state === 'unblocked') proceeding.current = false }, [blocker.state])
  useEffect(() => {
    if (!dirty) return
    const guard = (event: BeforeUnloadEvent) => { event.preventDefault(); event.returnValue = '' }
    window.addEventListener('beforeunload', guard)
    return () => window.removeEventListener('beforeunload', guard)
  }, [dirty])
  return <DraftContext value={register}>{children}<ConfirmDialog open={blocker.state === 'blocked'}
    onOpenChange={open => { if (!open && !proceeding.current && blocker.state === 'blocked') blocker.reset() }}
    title={lang === 'de' ? 'Ungespeicherte Änderungen verwerfen?' : 'Discard unsaved changes?'}
    description={lang === 'de' ? 'Deine Änderungen wurden noch nicht gespeichert. Bleibe hier, um sie zu speichern.' : 'Your changes have not been saved. Stay here to save them.'}
    confirmLabel={lang === 'de' ? 'Verwerfen und weiter' : 'Discard and continue'} cancelLabel={lang === 'de' ? 'Hier bleiben' : 'Stay here'}
    onConfirm={() => { if (blocker.state === 'blocked') { proceeding.current = true; setDrafts({}); blocker.proceed() } }} /></DraftContext>
}
