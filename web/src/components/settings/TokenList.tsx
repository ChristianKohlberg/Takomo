// The credential list.
//
// This was a single flex row per token — actor, every scope as a badge, the
// project list, a date and a button, all at one weight and wrapping wherever
// they ran out of room. Four tokens with the same actor name (which is what a
// real deployment looks like: one per machine, minted by the same script) were
// genuinely indistinguishable.
//
// So: a grid with real columns on desktop, stacked on mobile, and the identity
// column carrying the token's OWN id rather than only its actor — the id is what
// `takomo token revoke` takes, and what distinguishes two credentials that
// necessarily share an actor.
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type { TokenRow } from '@/lib/admin'

export interface TokenListLabels {
  scopes: string
  projects: string
  allProjects: string
  lastUsed: string
  neverUsed: string
  revoked: string
  expired: string
  revoke: string
  /** Marks the row belonging to the token the viewer is signed in with. */
  thisToken: string
}

export interface TokenListProps {
  tokens: TokenRow[]
  /** The signed-in token's id, so its own row can be marked and protected. */
  currentTokenId?: string
  labels: TokenListLabels
  onRevoke: (row: TokenRow) => void
}

function isExpired(row: TokenRow): boolean {
  return !!row.expires_at && new Date(row.expires_at).getTime() < Date.now()
}

export function TokenList({ tokens, currentTokenId, labels, onRevoke }: TokenListProps) {
  // Live first, then everything already dead. A revoked token is a record, not a
  // thing you act on, so it must not sit between two tokens that are.
  const ordered = [...tokens].sort((a, b) => {
    const dead = (r: TokenRow) => (r.revoked_at || isExpired(r) ? 1 : 0)
    return dead(a) - dead(b) || a.actor.localeCompare(b.actor)
  })

  return (
    // The column template lives HERE, on the list, with each row opting into it
    // via `grid-template-columns: subgrid`. One grid per row — the obvious
    // version — gives every row its own column widths, so rows whose content
    // differs in width do not line up: the viewer's own row has an extra badge
    // and no Revoke button, and it visibly stepped out of the column the other
    // rows shared. Subgrid is what makes a list of rows read as a table.
    <ul className="flex flex-col gap-px md:grid md:grid-cols-[minmax(0,1.4fr)_minmax(0,1.6fr)_auto_auto]">
      {ordered.map((row) => {
        const expired = isExpired(row)
        const dead = !!row.revoked_at || expired
        const isSelf = !!currentTokenId && row.id === currentTokenId

        return (
          <li
            key={row.id}
            className={cn(
              'bg-card border-border-soft grid grid-cols-1 items-center gap-x-4 gap-y-2 border px-3.5 py-3',
              'first:rounded-t-xl last:rounded-b-xl [&:not(:first-child)]:border-t-0',
              // Columns only from md up: below it everything stacks, because four
              // columns on a 320px screen is the collapse the grid-cols lint
              // exists to catch.
              'md:col-span-4 md:grid-cols-subgrid',
              dead && 'opacity-55',
            )}
          >
            {/* Identity */}
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="truncate font-mono text-[13px] font-[650]">{row.actor}</span>
                {isSelf && (
                  <Badge variant="outline" className="shrink-0">
                    {labels.thisToken}
                  </Badge>
                )}
              </div>
              <div className="text-muted-foreground mt-0.5 font-mono text-[11px]">{row.id}</div>
            </div>

            {/* Grants */}
            <div className="flex min-w-0 flex-wrap items-center gap-1">
              {row.scopes.map((s) => (
                <Badge key={s} variant="secondary">
                  {s}
                </Badge>
              ))}
              {row.projects === '*' ? (
                <span className="text-muted-foreground text-[11.5px]">{labels.allProjects}</span>
              ) : (
                row.projects.map((p) => (
                  <Badge key={p} variant="outline" className="font-mono">
                    {p}
                  </Badge>
                ))
              )}
            </div>

            {/* State */}
            <div className="text-muted-foreground text-[11.5px] whitespace-nowrap">
              {row.revoked_at ? (
                <Badge variant="destructive">{labels.revoked}</Badge>
              ) : expired ? (
                <Badge variant="destructive">{labels.expired}</Badge>
              ) : row.last_used_at ? (
                `${labels.lastUsed} ${row.last_used_at.slice(0, 10)}`
              ) : (
                labels.neverUsed
              )}
            </div>

            {/* Action. Present but inert on a dead row, so the column does not
                jump between rows — and absent entirely for the viewer's own
                token, because revoking it signs them out mid-task with no way
                back in. */}
            <div className="flex justify-start md:justify-end">
              {!dead && !isSelf && (
                <Button variant="destructive" size="sm" onClick={() => onRevoke(row)}>
                  {labels.revoke}
                </Button>
              )}
            </div>
          </li>
        )
      })}
    </ul>
  )
}
