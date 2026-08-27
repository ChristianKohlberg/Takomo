// The people directory, in /settings.
//
// It sits beside the credentials on purpose: those are the two halves of the same
// question. A credential says what may be done; a person says who work can be
// addressed to, and binding one to the other is an admin act with real
// consequence — a named assignee may answer an `approve`.
//
// Which is why a row shows the two facts an operator actually decides on: is this
// person still active, and which projects can hand them work. Everything else
// about them is a display name.
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type { User } from '@/lib/users'
import { Hint } from '@/components/Hint'

export interface PeopleListLabels {
  /** Column headings. */
  person: string
  projects: string
  noProjects: string
  status: string
  active: string
  disabled: string
  /** Row actions. */
  edit: string
  disable: string
  enable: string
  /** Tooltip on the disable action — it is not a delete. */
  disableHint: string
}

export interface PeopleListProps {
  people: User[]
  labels: PeopleListLabels
  /** Absent = a reader who cannot administer the directory; actions are hidden. */
  onSetDisabled?: (person: User, disabled: boolean) => void
  /** Open the dialog on this person. Hidden with the other actions when absent. */
  onEdit?: (person: User) => void
  busyHandle?: string
}

export function PeopleList({
  people,
  labels,
  onSetDisabled,
  onEdit,
  busyHandle,
}: PeopleListProps) {
  return (
    <ul className="flex flex-col gap-px">
      {people.map((p) => (
        <li
          key={p.id}
          className={cn(
            'border-b-border-soft flex flex-col gap-2 border-b py-3 last:border-b-0',
            // One breakpoint, `md`: stacked on a phone, columns from there up.
            'md:flex-row md:items-center md:gap-4',
            p.disabled && 'opacity-70',
          )}
        >
          <div className="min-w-0 md:w-64">
            <div className="flex items-baseline gap-2">
              <span className="truncate text-[13.5px] font-[680]">{p.label}</span>
              {p.disabled && (
                <Badge variant="outline" className="shrink-0">
                  {labels.disabled}
                </Badge>
              )}
            </div>
            {/* The handle, because it is what every `person:<handle>` reference
                and every CLI command takes — the display name is not addressable. */}
            <div className="text-muted-foreground truncate font-mono text-[11.5px]">
              {p.handle}
              {p.email ? ` · ${p.email}` : ''}
            </div>
          </div>

          <div className="text-muted-foreground min-w-0 grow text-[12.5px]">
            <span className="mr-1.5 font-[650]">{labels.projects}</span>
            {p.projects && p.projects.length > 0 ? (
              <span className="font-mono">{p.projects.join(', ')}</span>
            ) : (
              // Not a cosmetic gap: somebody who is a member of nothing cannot be
              // handed work anywhere, which is worth saying rather than leaving
              // as an empty cell.
              <span className="italic">{labels.noProjects}</span>
            )}
          </div>

          {(onEdit || onSetDisabled) && (
            <div className="flex shrink-0 gap-1.5 self-start md:self-auto">
              {onEdit && (
                <Button
                  variant="outline"
                  size="sm"
                  disabled={busyHandle === p.handle}
                  onClick={() => onEdit(p)}
                >
                  {labels.edit}
                </Button>
              )}
              {onSetDisabled && (
                <Hint text={p.disabled ? undefined : labels.disableHint}>
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={busyHandle === p.handle}
                    onClick={() => onSetDisabled(p, !p.disabled)}
                  >
                    {p.disabled ? labels.enable : labels.disable}
                  </Button>
                </Hint>
              )}
            </div>
          )}
        </li>
      ))}
    </ul>
  )
}
