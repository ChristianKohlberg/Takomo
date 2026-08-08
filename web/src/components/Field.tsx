// Label + control + optional hint, with the id wiring done once.
//
// The pages generated a random id per field to connect <label for> to its
// control. `useId` is React's answer to the same problem and is stable across
// renders, which the random one was not.
import { useId, type ReactNode } from 'react'
import { Label } from './ui/label'

export interface FieldProps {
  label: string
  hint?: string | null
  className?: string
  /** Receives the generated id — put it on the control. */
  children: (id: string) => ReactNode
}

export function Field({ label, hint, className, children }: FieldProps) {
  const id = useId()
  return (
    // Deliberately NO flex sizing of its own. A `flex: 1 1 170px` here reads as
    // a width in the row layouts and as a HEIGHT in the stacked ones, where it
    // padded every field out to 170px and left the dialogs full of gaps. The
    // row containers ask for it themselves via `[&>*]:flex-[1_1_170px]`.
    <div className={'flex min-w-0 flex-col gap-1 ' + (className ?? '')}>
      <Label
        htmlFor={id}
        className="text-muted-foreground text-[10.5px] font-bold tracking-[0.05em] uppercase"
      >
        {label}
      </Label>
      {children(id)}
      {hint && (
        <div className="text-muted-foreground text-[11px] font-medium normal-case">{hint}</div>
      )}
    </div>
  )
}
