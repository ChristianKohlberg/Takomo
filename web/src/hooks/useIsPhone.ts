// Is this a phone-width viewport?
//
// Nearly all of the responsive work in this app is CSS — a `md:` prefix costs
// nothing and cannot desynchronise from the stylesheet. This hook exists for the
// cases where the DIFFERENCE IS STRUCTURAL rather than visual: the board mounts
// only the selected state's column on a phone, so the other seven columns'
// cards are never rendered at all. Hiding them with `hidden` would still build
// every card, which on a large project is the majority of the render.
//
// The breakpoint is `md` (768px), and it is the ONLY breakpoint this app uses.
// Takomo has no tablet-specific design, so `md` means exactly "phone or not";
// see the responsive contract in web/README.md. Keep this number and Tailwind's
// `md` in step — they are the same line, expressed twice, which is the price of
// needing it in JS at all.
import { useEffect, useState } from 'react'

/** Tailwind's `md` breakpoint. Below this is "phone". */
export const PHONE_MAX = 767

const QUERY = `(max-width: ${PHONE_MAX}px)`

export function useIsPhone(): boolean {
  const [isPhone, setIsPhone] = useState(
    // Guarded for the non-browser render path (tests run in jsdom, which does
    // have matchMedia, but not every environment does).
    () => typeof window !== 'undefined' && !!window.matchMedia?.(QUERY).matches,
  )

  useEffect(() => {
    const mq = window.matchMedia?.(QUERY)
    if (!mq) return
    const on = () => setIsPhone(mq.matches)
    on()
    // Rotating a phone crosses this line, so it has to be live rather than
    // read once at mount.
    mq.addEventListener('change', on)
    return () => mq.removeEventListener('change', on)
  }, [])

  return isPhone
}
