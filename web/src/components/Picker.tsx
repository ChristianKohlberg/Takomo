// One choice from a short list — what `<select>` was doing, on the design
// system's own vocabulary instead of the browser's.
//
// WHY A WRAPPER, rather than composing Radix at each of the 26 call sites. A
// Radix select is five nested parts (`Select` → `SelectTrigger` → `SelectValue`
// → `SelectContent` → a `SelectItem` per option) where a native one was a tag
// and its children. Written longhand 26 times that is a great deal of identical
// scaffolding, and it buries the one thing that differs — the options — inside
// it.
//
// THE EMPTY STRING IS THE WHOLE REASON THIS IS NOT A MECHANICAL SWAP. A native
// `<option value="">All</option>` is the idiom this codebase uses for "no filter"
// in 12 places. Radix refuses it outright — `A <Select.Item /> must have a value
// prop that is not an empty string`, because it reserves `""` to mean "nothing is
// selected, show the placeholder". So `""` is mapped to a sentinel on the way in
// and back to `""` on the way out, HERE, once. Doing it at the call sites would
// mean 12 chances to leak `__any` into a query string or a request body.
//
// The sentinel is deliberately not a plausible id. Every value in this app is a
// slug, a tag, a user handle or a state name, and none of them can contain a
// space — so nothing real can ever collide with it.
//
// WHAT THIS COSTS, stated plainly: on a phone the OS no longer draws the picker.
// A native `<select>` opens the platform's own wheel or list, which is familiar,
// correctly sized, and works with the system's own assistive tooling. This is a
// styled listbox instead. Radix handles touch, focus and type-ahead properly, so
// it is not broken — but "looks like the rest of the app" is being bought with
// "behaves like the rest of the phone", and that is a real trade rather than a
// free upgrade.
import type { ReactNode } from 'react'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { cn } from '@/lib/utils'

/** The sentinel standing in for `""`. A space makes it uncollidable. */
const EMPTY = '__none __'

export interface PickerOption {
  /** `''` is allowed and is the "all"/"none" row; it is mapped internally. */
  value: string
  label: ReactNode
}

export interface PickerProps {
  value: string
  onValueChange: (value: string) => void
  options: readonly PickerOption[]
  /** Shown when `value` matches no option. */
  placeholder?: string
  id?: string
  className?: string
  disabled?: boolean
  'aria-label'?: string
}

export function Picker({
  value,
  onValueChange,
  options,
  placeholder,
  id,
  className,
  disabled,
  'aria-label': ariaLabel,
}: PickerProps) {
  return (
    <Select
      value={value === '' ? EMPTY : value}
      onValueChange={(v) => onValueChange(v === EMPTY ? '' : v)}
      disabled={disabled}
    >
      <SelectTrigger
        id={id}
        aria-label={ariaLabel}
        // `w-full` is not the primitive's default and is wanted at nearly every
        // call site here: these sit in filter bars and dialog fields that size
        // the control, where a native select stretched and this does not.
        className={cn('w-full', className)}
      >
        <SelectValue placeholder={placeholder} />
      </SelectTrigger>
      <SelectContent>
        {options.map((o) => (
          <SelectItem key={o.value === '' ? EMPTY : o.value} value={o.value === '' ? EMPTY : o.value}>
            {o.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
