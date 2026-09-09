import { createContext, useCallback, useContext, useEffect, useId, useMemo, useRef, useState, type ReactNode } from 'react'
import { useBlocker } from 'react-router'
import type { Locale } from '@/lib/i18n'
import { ConfirmDialog } from './ConfirmDialog'
interface Drafts { register: (id: string, dirty: boolean) => void; discard: () => void }
const DraftContext = createContext<Drafts | null>(null)
const noop = () => {}
/** Existing editors also render outside settings; no provider means no route guard. */
export function useSettingsDraft(dirty: boolean) {
  const register = useContext(DraftContext)?.register
  const id = useId()
  useEffect(() => { register?.(id, dirty); return () => register?.(id, false) }, [register, id, dirty])
}
/** Drops every registered draft, for a navigation the app makes once the data those drafts edit is gone. */
export function useDiscardDrafts() {
  return useContext(DraftContext)?.discard ?? noop
}
export function SettingsDrafts({ children, lang }: { children: ReactNode; lang: Locale }) {
  const live = useRef<Record<string, boolean>>({})
  const [drafts, setDrafts] = useState<Record<string, boolean>>({})
  const register = useCallback((id: string, dirty: boolean) => {
    if (dirty) live.current[id] = true
    else delete live.current[id]
    setDrafts(current => {
      if (!!current[id] === dirty) return current
      const next = { ...current }
      if (dirty) next[id] = true
      else delete next[id]
      return next
    })
  }, [])
  const discard = useCallback(() => { live.current = {}; setDrafts({}) }, [])
  const context = useMemo(() => ({ register, discard }), [register, discard])
  const proceeding = useRef(false)
  const dirty = Object.values(drafts).some(Boolean)
  const shouldBlock = useCallback(({ currentLocation, nextLocation }: { currentLocation: { pathname: string; search: string }; nextLocation: { pathname: string; search: string } }) =>
    Object.values(live.current).some(Boolean) && (currentLocation.pathname !== nextLocation.pathname || currentLocation.search !== nextLocation.search), [])
  const blocker = useBlocker(shouldBlock)
  useEffect(() => { if (blocker.state === 'unblocked') proceeding.current = false }, [blocker.state])
  useEffect(() => {
    if (!dirty) return
    const guard = (event: BeforeUnloadEvent) => { event.preventDefault(); event.returnValue = '' }
    window.addEventListener('beforeunload', guard)
    return () => window.removeEventListener('beforeunload', guard)
  }, [dirty])
  return <DraftContext value={context}>{children}<ConfirmDialog open={blocker.state === 'blocked'}
    onOpenChange={open => { if (!open && !proceeding.current && blocker.state === 'blocked') blocker.reset() }}
    title={lang === 'de' ? 'Ungespeicherte Änderungen verwerfen?' : 'Discard unsaved changes?'}
    description={lang === 'de' ? 'Deine Änderungen wurden noch nicht gespeichert. Bleibe hier, um sie zu speichern.' : 'Your changes have not been saved. Stay here to save them.'}
    confirmLabel={lang === 'de' ? 'Verwerfen und weiter' : 'Discard and continue'} cancelLabel={lang === 'de' ? 'Hier bleiben' : 'Stay here'}
    onConfirm={() => { if (blocker.state === 'blocked') { proceeding.current = true; discard(); blocker.proceed() } }} /></DraftContext>
}
