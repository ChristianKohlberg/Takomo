import { useCallback, useLayoutEffect, useRef, useState } from 'react'
/** Emit user/focus actions, never an old selection just because a view reappears.
 *  The third element sets the selection WITHOUT emitting, for a caller that is
 *  about to write the same target into the URL itself in one navigation. */
export function usePersonalSelection(onChange?: (id: string | null) => void) {
  const [selected, setSelected] = useState<string | null>(null)
  const callback = useRef(onChange)
  useLayoutEffect(() => {
    callback.current = onChange
  }, [onChange])
  const select = useCallback((id: string | null) => {
    setSelected(id)
    callback.current?.(id)
  }, [])
  return [selected, select, setSelected] as const
}
