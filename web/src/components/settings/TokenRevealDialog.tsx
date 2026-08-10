// The one and only time the plaintext token is shown.
//
// This was a panel inside the card, below the form, above the list — dismissible
// by scrolling past it, and easy to lose entirely when the list refreshed under
// it. Takomo stores only a SHA-256, so a token missed here is not recoverable by
// any means: the only remedy is to revoke it and mint another.
//
// A modal that must be acknowledged is therefore not friction, it is the
// feature. It cannot be dismissed by clicking outside for the same reason.
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

export interface TokenRevealDialogLabels {
  title: string
  subtitle: string
  copy: string
  copied: string
  done: string
}

export interface TokenRevealDialogProps {
  /** The plaintext, or `null` when there is nothing to reveal. */
  token: string | null
  actor?: string
  labels: TokenRevealDialogLabels
  onClose: () => void
}

export function TokenRevealDialog({ token, actor, labels, onClose }: TokenRevealDialogProps) {
  const [copied, setCopied] = useState(false)

  async function copy() {
    if (!token) return
    try {
      await navigator.clipboard.writeText(token)
      setCopied(true)
    } catch {
      // Clipboard access can be refused (permissions, an insecure origin). The
      // token is on screen and selectable, so this is recoverable by hand —
      // silently leaving the button unchanged says "that did not work" without
      // claiming a success that did not happen.
    }
  }

  return (
    <Dialog
      open={token != null}
      onOpenChange={(o) => {
        if (!o) {
          setCopied(false)
          onClose()
        }
      }}
    >
      <DialogContent
        className="max-w-[calc(100%-2rem)] sm:max-w-124"
        // Not dismissible by an outside click or Escape: this value cannot be
        // shown again, and both are things people do reflexively.
        onPointerDownOutside={(e) => e.preventDefault()}
        onEscapeKeyDown={(e) => e.preventDefault()}
      >
        <DialogHeader>
          <DialogTitle>{labels.title}</DialogTitle>
          <DialogDescription>{labels.subtitle}</DialogDescription>
        </DialogHeader>

        {actor && <div className="text-muted-foreground font-mono text-[12px]">{actor}</div>}

        <code className="bg-muted text-foreground block rounded-lg px-3 py-2.5 font-mono text-[12.5px] break-all select-all">
          {token}
        </code>

        <DialogFooter>
          <Button variant="secondary" onClick={() => void copy()}>
            {copied ? labels.copied : labels.copy}
          </Button>
          <Button
            onClick={() => {
              setCopied(false)
              onClose()
            }}
          >
            {labels.done}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
