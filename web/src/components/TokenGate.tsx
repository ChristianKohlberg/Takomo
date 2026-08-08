// The token gate: the page is unauthenticated, every data fetch carries the
// bearer token the viewer supplies here, and it is kept in localStorage per
// origin. Serving the HTML leaks nothing the API does not already guard.
import { useState, type FormEvent } from 'react'
import { Logo } from './Logo'
import { Button } from './ui/button'
import { Input } from './ui/input'
import { Label } from './ui/label'

export interface TokenGateProps {
  title: string
  subtitle: string
  tokenLabel: string
  openLabel: string
  /** Shown under the field — a rejected token, or a missing one. */
  error?: string
  /** Pre-fills the field when a stored token was rejected. */
  initialToken?: string
  emptyMessage: string
  onSubmit: (token: string) => void
}

export function TokenGate({
  title,
  subtitle,
  tokenLabel,
  openLabel,
  error,
  initialToken = '',
  emptyMessage,
  onSubmit,
}: TokenGateProps) {
  const [value, setValue] = useState(initialToken)
  const [localError, setLocalError] = useState('')

  function submit(e: FormEvent) {
    e.preventDefault()
    const tk = value.trim()
    if (!tk) {
      setLocalError(emptyMessage)
      return
    }
    setLocalError('')
    onSubmit(tk)
  }

  return (
    <div className="bg-background fixed inset-0 z-80 flex items-center justify-center">
      <form
        onSubmit={submit}
        className="bg-card w-full max-w-100 rounded-[14px] px-8 py-7.5 shadow-[var(--shadow)]"
      >
        <div className="text-[color:var(--accent2)]">
          <Logo />
        </div>
        <h1 className="mt-3 mb-1 text-[19px] font-[740] tracking-[-0.02em]">{title}</h1>
        <p className="text-muted-foreground mb-4.5 text-[13px]">{subtitle}</p>

        <Label htmlFor="g-token" className="text-muted-foreground mb-1.5 block text-[10.5px] font-bold tracking-[0.05em] uppercase">
          {tokenLabel}
        </Label>
        <Input
          id="g-token"
          type="password"
          autoComplete="off"
          placeholder="tk_live_••••••••"
          className="font-mono"
          value={value}
          onChange={(e) => setValue(e.target.value)}
        />

        <Button type="submit" className="mt-3.5 w-full">
          {openLabel}
        </Button>
        <div className="text-destructive mt-2.5 min-h-4 text-[12.5px]">{localError || error}</div>
      </form>
    </div>
  )
}
