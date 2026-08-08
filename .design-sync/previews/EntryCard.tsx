import { EntryCard } from '@takomo/web'

const noop = () => {}
const ago = (ms: number) => new Date(Date.now() - ms).toISOString()
const LABELS = { by: 'by', wrote: 'written', download: 'Download' }
const wrap: React.CSSProperties = { maxWidth: 640 }

/**
 * The common case: an agent's research entry. Every entry records WHERE IT CAME
 * FROM — source, optional origin time, author — which is what makes the
 * collection weighable later rather than an undifferentiated pile of text.
 */
export function Research() {
  return (
    <div style={wrap}>
      <EntryCard
        entry={{
          id: 'ie-0pdrl9iq',
          initiative: 'ini-tm41jq69',
          kind: 'research',
          source: 'claude:chat',
          title: 'What the rollup double-counts',
          text: '`rollup_for_epic` counts **all** descendants, with no type filter.\n\n- a sub-epic counts as one unit of work in its parent\n- its own work is counted twice across two rows',
          created_at: ago(5 * 60_000),
          author: 'human:ada',
        }}
        labels={LABELS}
        onDownload={noop}
      />
    </div>
  )
}

/** A short note from a worker agent — the smallest useful entry. */
export function Note() {
  return (
    <div style={wrap}>
      <EntryCard
        entry={{
          id: 'ie-1a2b3c4d',
          initiative: 'ini-tm41jq69',
          kind: 'note',
          source: 'agent:w1',
          text: 'Claimed the ticket and started the port.',
          created_at: ago(2 * 3600_000),
          author: 'agent:w1',
        }}
        labels={LABELS}
        onDownload={noop}
      />
    </div>
  )
}

/** An attachment: entries are the only thing in the store that holds bytes. */
export function WithAttachment() {
  return (
    <div style={wrap}>
      <EntryCard
        entry={{
          id: 'ie-9z8y7x6w',
          initiative: 'ini-tm41jq69',
          kind: 'document',
          source: 'person:ada',
          title: 'Roadmap review notes',
          text: 'The version we walked through in the review.',
          created_at: ago(26 * 3600_000),
          author: 'human:ada',
          has_content: true,
          filename: 'roadmap-review.pdf',
          content_bytes: 284_173,
        }}
        labels={LABELS}
        onDownload={noop}
      />
    </div>
  )
}

/** Provenance in full: a source link and an origin older than the entry itself. */
export function WithSourceAndOrigin() {
  return (
    <div style={wrap}>
      <EntryCard
        entry={{
          id: 'ie-5t6r7e8w',
          initiative: 'ini-tm41jq69',
          kind: 'feedback',
          source: 'person:sam',
          title: 'From the customer call',
          text: 'They want the roadmap to show sub-epics indented, not flattened.',
          source_uri: 'https://github.com/ChristianKohlberg/Takomo',
          origin_at: ago(12 * 86_400_000),
          created_at: ago(3 * 86_400_000),
          author: 'human:sam',
        }}
        labels={LABELS}
        onDownload={noop}
      />
    </div>
  )
}
