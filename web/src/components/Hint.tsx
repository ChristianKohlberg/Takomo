// A hover/focus hint on one control — what `title="…"` was doing, done in a way
// a touch device and a screen reader can both reach.
//
// WHY THIS EXISTS AT ALL, rather than composing Radix at each call site. A
// tooltip is four nested elements (`Tooltip` → `TooltipTrigger asChild` →
// the control → `TooltipContent`), and there are 47 of these. Written out longhand
// that turns a one-attribute change into a four-line restructure 47 times, and
// every one of those is a chance to nest the trigger wrong. Here the call site
// stays one wrap:
//
//     <Hint text={t.refreshHint}><Button …/></Hint>
//
// WHAT IT FIXES. `title` has never worked on a touch device — there is no hover,
// so the text is simply unreachable on a phone, which is half the traffic these
// surfaces are built for. It is also slow (a ~1s browser delay nobody can tune),
// unstyleable, and truncated by some browsers. Radix gives keyboard focus
// parity, `aria-describedby` wiring, and a delay this app controls.
//
// WHAT IT DELIBERATELY DOES NOT DO: set `aria-label`. A tooltip DESCRIBES a
// control; it does not NAME one. Radix already wires `aria-describedby`, which is
// the correct relationship. Overwriting the name would be actively wrong on a
// control that has visible text — the name would stop matching what is written on
// it, which is the "label in name" failure. An icon-only control still needs its
// own `aria-label`, exactly as it did before, and `Hint` is not a substitute for
// one.
//
// An empty or absent `text` renders the child alone, so a conditional hint does
// not need a conditional wrapper at the call site.
import type { ReactElement, ReactNode } from 'react'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'

export interface HintProps {
  /** The hint. Empty or undefined renders `children` with no tooltip at all. */
  text?: ReactNode
  /** Which side to prefer; Radix flips it when there is no room. */
  side?: 'top' | 'right' | 'bottom' | 'left'
  /** Exactly one element — it becomes the trigger via `asChild`. */
  children: ReactElement
}

export function Hint({ text, side = 'top', children }: HintProps) {
  if (text === undefined || text === null || text === '') return children
  return (
    // The provider lives HERE rather than once at the app root, and that is a
    // deliberate trade with a cost worth stating.
    //
    // Radix throws "`Tooltip` must be used within `TooltipProvider`" with no
    // ancestor provider, so a root-level one makes every component containing a
    // Hint unrenderable on its own. That is not hypothetical: it took out 26
    // NavRail tests at once, and it would do the same to any design-system
    // consumer importing NavRail from the barrel — which is precisely the
    // "takes everything it needs" rule that barrel documents. A component that
    // only works inside our own main.tsx is not a component we can export.
    //
    // What it costs: `skipDelayDuration`, which lets a SECOND hint open
    // instantly once a first one has, only groups within a provider. Per-Hint
    // providers mean each hint waits its own delay, so a row of icon buttons
    // re-waits at every step. The delay is set below the shadcn default to keep
    // that from feeling slow; it renders no DOM, so nesting them is free.
    <TooltipProvider delayDuration={250}>
      <Tooltip>
        <TooltipTrigger asChild>{children}</TooltipTrigger>
        <TooltipContent side={side}>{text}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  )
}
