import { Logo } from '@takomo/web'

/**
 * The takomo mark — a waveform. It inherits `currentColor`, so it takes the tone
 * of whatever it sits in rather than carrying its own.
 */
export function Sizes() {
  return (
    <div style={{ display: 'flex', gap: 18, alignItems: 'center', color: 'var(--accent2)' }}>
      <Logo size={16} />
      <Logo size={22} />
      <Logo size={36} />
      <Logo size={56} />
    </div>
  )
}

/** In the brand lockup, which is how every surface actually shows it. */
export function Brand() {
  return (
    <div style={{ display: 'flex', gap: 10, alignItems: 'center', color: 'var(--accent2)' }}>
      <Logo />
      <span style={{ fontSize: 16, fontWeight: 750, letterSpacing: '-.02em', color: 'var(--text)' }}>
        takomo
      </span>
    </div>
  )
}

/** It takes the ink of its context — the same mark, three tones. */
export function Tones() {
  return (
    <div style={{ display: 'flex', gap: 18, alignItems: 'center' }}>
      <span style={{ color: 'var(--accent)' }}>
        <Logo size={28} />
      </span>
      <span style={{ color: 'var(--muted)' }}>
        <Logo size={28} />
      </span>
      <span style={{ color: 'var(--crit)' }}>
        <Logo size={28} />
      </span>
    </div>
  )
}
