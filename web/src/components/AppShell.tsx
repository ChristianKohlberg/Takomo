// The frame every surface sits in: the nav rail on the left, the page to the
// right of it.
//
// Each page used to open with the same `flex h-dvh flex-col overflow-hidden`
// root and then render its own header. That root is now here, so adding the
// rail did not mean editing five layouts — and the next thing that belongs to
// the whole app rather than to one surface has one place to go.
//
// `min-w-0` on the content column is load-bearing rather than tidy: a flex item
// defaults to `min-width: auto`, so a wide table or a long unbroken ticket title
// inside it would refuse to shrink and push the page sideways instead of
// scrolling within itself.
import type { ReactNode } from 'react'
import { NavRail, type NavRailProps } from './NavRail'

export interface AppShellProps {
  /** Everything the rail needs; see NavRail. */
  rail: NavRailProps
  /** The surface: its header and its body. */
  children: ReactNode
}

export function AppShell({ rail, children }: AppShellProps) {
  return (
    <div className="flex h-dvh overflow-hidden">
      <NavRail {...rail} />
      <div className="flex min-w-0 grow flex-col overflow-hidden">{children}</div>
    </div>
  )
}
