// Is the nav rail collapsed to icons? A viewer preference, so it is remembered
// per origin and shared by all five surfaces — collapsing it on /board and
// finding it expanded again on /inbox would read as a bug.
//
// It lives outside NavRail so the component stays props-only: that is the
// contract for anything in the design-system barrel, and it is what lets the
// rail render in a preview with no localStorage at all.
import { useCallback, useEffect, useState } from 'react'
import { useIsPhone } from './useIsPhone'

const LS_KEY = 'takomo.nav.collapsed'

export function useNavCollapsed(): [boolean, (collapsed: boolean) => void] {
  const isPhone = useIsPhone()
  const [stored, setStored] = useState(
    () => typeof localStorage !== 'undefined' && localStorage.getItem(LS_KEY) === '1',
  )
  // A phone starts collapsed whatever the stored preference says: expanded
  // there is an overlay covering the page, which is not a state to open into.
  const [collapsed, setCollapsed] = useState(() => isPhone || stored)

  useEffect(() => {
    if (isPhone) setCollapsed(true)
    else setCollapsed(stored)
  }, [isPhone, stored])

  const update = useCallback(
    (next: boolean) => {
      setCollapsed(next)
      // Opening the overlay on a phone is a momentary act, not a preference —
      // persisting it would expand the rail on the next desktop visit.
      if (isPhone) return
      setStored(next)
      try {
        localStorage.setItem(LS_KEY, next ? '1' : '0')
      } catch {
        // Private mode, or storage full. The rail still toggles for this visit.
      }
    },
    [isPhone],
  )

  return [collapsed, update]
}
