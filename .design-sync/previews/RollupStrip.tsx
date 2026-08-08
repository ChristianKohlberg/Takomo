import { RollupStrip } from '@takomo/web'

const LABELS = {
  entries: 'Entries',
  attachments: 'Attachments',
  chars: 'Characters',
  size: 'Size',
  last: 'Last entry',
}

/**
 * What has accumulated on an initiative — the case for it being worth keeping.
 * Sizes get their real unit: the API sends `megabytes`, but a 300-byte note
 * would read as "0 MB".
 */
export function Populated() {
  return (
    <div style={{ maxWidth: 720 }}>
      <RollupStrip
        rollup={{
          entries: 14,
          attachments: 3,
          chars: 48_211,
          bytes: 1_284_173,
          last_entry_at: new Date(Date.now() - 3 * 3600_000).toISOString(),
        }}
        labels={LABELS}
      />
    </div>
  )
}

/** A fresh initiative: zeroes, and an em dash rather than a fake timestamp. */
export function Empty() {
  return (
    <div style={{ maxWidth: 720 }}>
      <RollupStrip rollup={{ entries: 0, attachments: 0, chars: 0, bytes: 0 }} labels={LABELS} />
    </div>
  )
}

/** Small sizes keep bytes, which is why the formatter exists. */
export function SmallSizes() {
  return (
    <div style={{ maxWidth: 720 }}>
      <RollupStrip
        rollup={{
          entries: 1,
          attachments: 0,
          chars: 300,
          bytes: 300,
          last_entry_at: new Date(Date.now() - 45_000).toISOString(),
        }}
        labels={LABELS}
      />
    </div>
  )
}

/** German labels — the component renders whatever it is handed. */
export function German() {
  return (
    <div style={{ maxWidth: 720 }}>
      <RollupStrip
        rollup={{ entries: 14, attachments: 3, chars: 48_211, bytes: 1_284_173 }}
        labels={{
          entries: 'Einträge',
          attachments: 'Anhänge',
          chars: 'Zeichen',
          size: 'Größe',
          last: 'Letzter Eintrag',
        }}
      />
    </div>
  )
}
