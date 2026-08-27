// The ticket drawer.
//
// Each attribute is encoded once: the id and the claim holder are monospace
// because they are identifiers, priority is a single coloured word, and the
// commit link shows a short sha with the full value on hover — the commit is the
// proof a "done" claim rests on, so it has to stay readable rather than wrap a
// 40-character sha across the drawer.
import { Markdown } from '@/components/Markdown'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { cn } from '@/lib/utils'
import { fmtAge } from '@/lib/format'
import type { Ticket } from '@/lib/board'
import { Hint } from '@/components/Hint'

const PRIORITY: Record<string, string> = {
  critical: 'text-crit',
  high: 'text-high',
  normal: 'text-normal',
  low: 'text-low',
}

/** A sha stays legible as its stub; the full value lives in the title. */
function shortCommit(v: string): string {
  const m = /([0-9a-f]{7,40})$/i.exec(v.trim())
  return m ? m[1]!.slice(0, 10) : v
}

export interface OpenQuestions {
  count: number
  blocking: number
  advisory: number
  /** Bounced back to the agent: there is nothing for the reader to answer yet. */
  conv: number
}

export interface DetailPanelLabels {
  state: string
  claimedBy: string
  labels: string
  tagsHdr: string
  description: string
  noDescription: string
  dependencies: string
  blockedByRel: string
  links: string
  blockedN: string
  answeringResumes: string
  decisionRouted: string
  answerInInbox: string
  inConvN: string
  inConvSub: string
  readThread: string
  askHuman: string
  close: string
  promotions: string
  comments: string
  noComments: string
  refLabel: string
  agoSep: string
}

export interface DetailPanelProps {
  ticket: Ticket | null
  questions?: OpenQuestions
  labels: DetailPanelLabels
  canAsk: boolean
  onClose: () => void
  onAsk: () => void
  /** Client-side navigation for the inbox link; see AppHeader.onNavigate. */
  onNavigate?: (href: string) => void
}

export function DetailPanel({
  ticket: t,
  questions,
  labels,
  canAsk,
  onClose,
  onAsk,
  onNavigate,
}: DetailPanelProps) {
  if (!t) return null

  // "Answering resumes this ticket" is a lie when every open question has been
  // bounced back to the agent — there is nothing to answer until it reports
  // back, and the reader's job here is to read.
  const convOnly = !!questions && questions.conv > 0 && !questions.blocking && !questions.advisory

  return (
    // A real dialog now, not an <aside> under a hand-rolled overlay. Radix
    // supplies the focus trap, focus restore, Escape, `role="dialog"`,
    // `aria-modal` and the `aria-labelledby` wiring from DialogTitle — every one
    // of which was missing before, on the panel this board opens most.
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent
        side="right"
        className="bg-card border-border gap-0 p-0 shadow-[-24px_0_60px_-30px_rgba(20,40,55,.5)]"
      >
        <DialogHeader className="bg-card border-b-border-soft sticky top-0 z-1 flex-row items-start gap-2 border-b px-6 pt-5 pb-4">
          <DialogTitle className="m-0 flex-1 text-[20px] leading-[1.3] font-[720] tracking-[-0.02em] break-words">
            {t.title || t.id}
          </DialogTitle>
        </DialogHeader>

        <div className="flex flex-col gap-5 px-6 py-4">
          {questions && questions.count > 0 && (
            <div className={cn('rounded-[11px] px-4 py-3.5', convOnly ? 'bg-muted' : 'bg-secondary')}>
              <div className={cn('mb-1.5 text-[12px] font-bold', convOnly ? 'text-foreground' : 'text-primary')}>
                {(convOnly ? labels.inConvN : labels.blockedN).replace(
                  '{n}',
                  String(convOnly ? questions.conv : questions.count),
                )}
              </div>
              <div className="text-foreground mb-2.75 text-[13px] leading-[1.5]">
                {convOnly
                  ? labels.inConvSub
                  : questions.blocking > 0
                    ? labels.answeringResumes
                    : labels.decisionRouted}
              </div>
              <a
                href="/inbox"
                onClick={(e) => {
                  // Same rule as the header nav: intercept only a plain
                  // left-click, so cmd-click still opens the inbox in a new tab.
                  if (!onNavigate) return
                  if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return
                  e.preventDefault()
                  onNavigate('/inbox')
                }}
                className="bg-primary text-primary-foreground inline-flex items-center gap-1.5 rounded-lg px-3.5 py-2 text-[13px] font-[680] no-underline"
              >
                {convOnly ? labels.readThread : labels.answerInInbox}
              </a>
            </div>
          )}

          <div className="text-muted-foreground flex flex-wrap items-baseline gap-2 text-[12.5px]">
            <span className="text-foreground font-mono">{t.id}</span>
            <span className={cn('font-[680]', PRIORITY[t.priority ?? 'low'] ?? 'text-low')}>
              {t.priority ?? '—'}
            </span>
            <span>
              {labels.state}: {t.state}
            </span>
            {t.type && <span>{t.type}</span>}
            {t.claim?.holder && (
              <span>
                {labels.claimedBy} <span className="text-foreground font-mono">{t.claim.holder}</span>
              </span>
            )}
          </div>

          {!!t.labels?.length && (
            <Section title={labels.labels}>
              <div className="text-muted-foreground flex flex-wrap gap-2 text-[12.5px]">
                {t.labels.map((l) => (
                  <span key={l}>{l}</span>
                ))}
              </div>
            </Section>
          )}

          {!!t.tags?.length && (
            <Section title={labels.tagsHdr}>
              <div className="text-muted-foreground flex flex-wrap gap-2 font-mono text-[12.5px]">
                {t.tags.map((tag) => (
                  <span key={tag}>{tag}</span>
                ))}
              </div>
            </Section>
          )}

          <Section title={labels.description}>
            {t.body ? (
              <Markdown text={t.body} className="text-[13.5px] leading-[1.65]" />
            ) : (
              <div className="text-muted-foreground text-[13.5px]">{labels.noDescription}</div>
            )}
          </Section>

          {!!t.blocked_by?.length && (
            <Section title={labels.dependencies}>
              {t.blocked_by.map((d) => (
                <div key={d} className="text-muted-foreground text-[12.5px]">
                  {labels.blockedByRel}
                  <span className="text-foreground font-mono">{d}</span>
                </div>
              ))}
            </Section>
          )}

          {t.links && Object.keys(t.links).length > 0 && (
            <Section title={labels.links}>
              {Object.entries(t.links).map(([k, v]) => {
                const isCommit = k === 'commit' && typeof v === 'string' && v.trim() !== ''
                const text = isCommit ? shortCommit(v) : v
                return (
                  <div key={k} className="text-[12.5px]">
                    <strong>{k}: </strong>
                    {/^https?:\/\//.test(v) ? (
                      <Hint text={isCommit ? v : undefined}>
                        <a
                          href={v}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="text-[color:var(--accent2)] underline"
                        >
                          {text}
                        </a>
                      </Hint>
                    ) : (
                      <Hint text={isCommit ? v : undefined}>
                        <span className="font-mono">
                          {text}
                        </span>
                      </Hint>
                    )}
                  </div>
                )
              })}
            </Section>
          )}

          {!!t.promotions?.length && (
            <Section title={labels.promotions}>
              {/* Newest first: a promotion is a claim about where this work
                  reached, and the most recent one is the live answer. */}
              {t.promotions.map((p, i) => (
                <div key={i} className="mb-2 text-[12.5px]">
                  <div className="flex flex-wrap items-baseline gap-2">
                    <span className="text-foreground font-mono font-[650]">{p.target}</span>
                    <span className="text-muted-foreground">
                      {fmtAge(p.created_at)}
                      {labels.agoSep}
                      {p.actor}
                    </span>
                  </div>
                  {(p.url || p.ref || p.note) && (
                    <div className="text-muted-foreground mt-0.5 flex flex-wrap gap-1.5">
                      {p.url &&
                        (/^https?:\/\//.test(p.url) ? (
                          <a
                            href={p.url}
                            target="_blank"
                            rel="noopener noreferrer"
                            className="text-[color:var(--accent2)] underline [overflow-wrap:anywhere]"
                          >
                            {p.url}
                          </a>
                        ) : (
                          <span>{p.url}</span>
                        ))}
                      {p.ref && (
                        <span>
                          {labels.refLabel}
                          {p.ref}
                        </span>
                      )}
                      {p.note && <span>{p.note}</span>}
                    </div>
                  )}
                </div>
              ))}
            </Section>
          )}

          <Section title={`${labels.comments} (${t.comments?.length ?? 0})`}>
            {t.comments?.length ? (
              t.comments.map((c, i) => (
                <div key={i} className="border-border mb-2.5 border-l-2 py-0.5 pl-2.5">
                  <div className="text-muted-foreground font-mono text-[11px]">
                    {c.author} · {fmtAge(c.created_at)}
                  </div>
                  <Markdown text={c.body} className="text-[13px]" />
                </div>
              ))
            ) : (
              <div className="text-muted-foreground text-[13px]">{labels.noComments}</div>
            )}
          </Section>

          {canAsk && (
            <Button variant="outline" onClick={onAsk} className="w-fit">
              {labels.askHuman}
            </Button>
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section>
      <h3 className="text-muted-foreground m-0 mb-2 text-[12px] font-bold tracking-[0.06em] uppercase">
        {title}
      </h3>
      {children}
    </section>
  )
}
