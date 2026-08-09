// The minted answer link, shown once.
//
// This is what an outside expert gets: a scoped, expiring, single-use URL that
// lets them answer exactly this one question — no token of their own, no access
// to anything else. It is shown once because the server does not keep the
// plaintext, so "copy it now" is a fact rather than a nag.
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { fmtWhen } from '@/lib/cadence'
import type { Locale } from '@/lib/i18n'
import type { AnswerLink } from '@/lib/questions'

export interface AnswerLinkDialogProps {
  link: AnswerLink | null
  /** The expiry is read by a person, so it is formatted, not printed raw. */
  lang: Locale
  onClose: () => void
  labels: {
    title: string
    body: string
    once: string
    copy: string
    copied: string
    done: string
    validUntil: string
    copyFail: string
  }
}

export function AnswerLinkDialog({ link, lang, onClose, labels }: AnswerLinkDialogProps) {
  const [copied, setCopied] = useState(false)
  const [failed, setFailed] = useState(false)
  const url = link?.url ?? link?.token ?? ''

  async function copy() {
    try {
      await navigator.clipboard.writeText(url)
      setCopied(true)
      setFailed(false)
    } catch {
      // Clipboard access can be refused outright; say so rather than silently
      // appearing to have copied.
      setFailed(true)
    }
  }

  return (
    <Dialog open={!!link} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-[calc(100%-2rem)] sm:max-w-140">
        <DialogHeader>
          <DialogTitle>{labels.title}</DialogTitle>
          <DialogDescription>{labels.body}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-2">
          <div className="text-muted-foreground text-[12px] font-[650]">{labels.once}</div>
          <div className="border-border bg-muted overflow-x-auto rounded-lg border px-3 py-2 font-mono text-[12px] break-all">
            {url}
          </div>
          {link?.expires_at && (
            <div className="text-muted-foreground font-mono text-[11.5px]">
              {labels.validUntil} {fmtWhen(link.expires_at, lang)}
            </div>
          )}
          {failed && <div className="text-destructive text-[12.5px]">{labels.copyFail}</div>}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={copy}>
            {copied ? labels.copied : labels.copy}
          </Button>
          <Button onClick={onClose}>{labels.done}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
