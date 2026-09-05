import { useCallback, useLayoutEffect, useRef, useState } from 'react'
/** Emit user/focus actions, never an old selection just because a view reappears. */
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
  return [selected, select] as const
}
