// Toasts, ported from the pages' own implementation.
//
// Deliberately NOT shadcn's `sonner`: that is another runtime dependency, and
// every dependency here is inlined into four documents with no shared cache.
// This is ~40 lines and does what the pages already did — a message, an error or
// success tone, dismiss on click, auto-dismiss after 5s.
import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from 'react'
import { cn } from '@/lib/utils'

export type ToastKind = 'err' | 'success'

interface Toast {
  id: number
  message: string
  kind: ToastKind
}

interface ToastApi {
  toast: (message: string, kind?: ToastKind) => void
}

const ToastContext = createContext<ToastApi | null>(null)

export function useToast(): ToastApi {
  const ctx = useContext(ToastContext)
  if (!ctx) throw new Error('useToast must be used inside <ToastProvider>')
  return ctx
}

let nextId = 1

export function ToastProvider({
  children,
  dismissLabel = 'Dismiss',
}: {
  children: ReactNode
  dismissLabel?: string
}) {
  const [toasts, setToasts] = useState<Toast[]>([])

  const remove = useCallback((id: number) => {
    setToasts((all) => all.filter((t) => t.id !== id))
  }, [])

  const toast = useCallback(
    (message: string, kind: ToastKind = 'err') => {
      const id = nextId++
      setToasts((all) => [...all, { id, message, kind }])
      setTimeout(() => remove(id), 5000)
    },
    [remove],
  )

  const api = useMemo(() => ({ toast }), [toast])

  return (
    <ToastContext.Provider value={api}>
      {children}
      <div className="fixed right-4 bottom-[calc(1rem+env(safe-area-inset-bottom))] z-90 flex flex-col gap-2">
        {toasts.map((t) => (
          <div
            key={t.id}
            role="status"
            className={cn(
              'flex max-w-[min(25rem,calc(100vw-2rem))] items-start gap-2.5 rounded-[10px] px-3.5 py-3 text-[12.8px] shadow-[0_18px_40px_-18px_rgba(0,0,0,.5)]',
              t.kind === 'success' ? 'bg-ok text-white' : 'bg-foreground text-background',
            )}
          >
            <div className="grow">{t.message}</div>
            <button
              type="button"
              aria-label={dismissLabel}
              className="cursor-pointer border-0 bg-transparent text-[15px] leading-none opacity-70"
              onClick={() => remove(t.id)}
            >
              ×
            </button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  )
}
