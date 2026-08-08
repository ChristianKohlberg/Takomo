import { Composer } from '@takomo/web'

// The state-heavy one: a fully controlled form. Every cell pins the state
// explicitly rather than wiring handlers, so the card renders the state it
// claims to show and nothing depends on interaction.

const LABELS = {
  kind: 'Kind',
  kindHint: 'note, research, feedback, transcript, document …',
  source: 'Source',
  sourceHint: 'Required — where did this come from? agent:w1, person:ada, claude:chat',
  title: 'Heading',
  titlePh: 'optional',
  uri: 'Link',
  uriPh: 'https://… (optional)',
  text: 'Text',
  textPh: 'Markdown. A note, a finding, feedback …',
  origin: 'Written at',
  originHint: 'optional — when the content is older than now',
  attach: 'Attach a file',
  attachClear: 'remove',
  attachAria: 'Attach a document to this entry',
  append: 'Append',
  appending: 'Appending …',
}

const EMPTY = { kind: 'note', source: '', title: '', text: '', uri: '', origin: '' }
const noop = () => {}

/** Ready to type: kind pre-filled, source defaulted to who you are. */
export function Empty() {
  return (
    <div style={{ maxWidth: 640 }}>
      <Composer
        draft={{ ...EMPTY, source: 'human:ada' }}
        onDraft={noop}
        file={null}
        onPickFile={noop}
        busy={false}
        onAppend={noop}
        labels={LABELS}
      />
    </div>
  )
}

/** A filled draft — the common case, an agent recording a finding. */
export function Filled() {
  return (
    <div style={{ maxWidth: 640 }}>
      <Composer
        draft={{
          kind: 'research',
          source: 'claude:chat',
          title: 'What the rollup double-counts',
          text: '`rollup_for_epic` counts **all** descendants, with no type filter.',
          uri: 'https://github.com/ChristianKohlberg/Takomo',
          origin: '',
        }}
        onDraft={noop}
        file={null}
        onPickFile={noop}
        busy={false}
        onAppend={noop}
        labels={LABELS}
      />
    </div>
  )
}

/** A picked attachment: the name and its real size replace the file input. */
export function WithAttachment() {
  return (
    <div style={{ maxWidth: 640 }}>
      <Composer
        draft={{
          kind: 'document',
          source: 'person:ada',
          title: 'Roadmap review notes',
          text: '',
          uri: '',
          origin: '',
        }}
        onDraft={noop}
        file={{ name: 'roadmap-review.pdf', mime: 'application/pdf', b64: '', size: 284_173 }}
        onPickFile={noop}
        busy={false}
        onAppend={noop}
        labels={LABELS}
      />
    </div>
  )
}

/** In flight: the append button locks so a write cannot double-fire. */
export function Busy() {
  return (
    <div style={{ maxWidth: 640 }}>
      <Composer
        draft={{
          kind: 'note',
          source: 'agent:w1',
          title: '',
          text: 'Claimed the ticket and started the port.',
          uri: '',
          origin: '',
        }}
        onDraft={noop}
        file={null}
        onPickFile={noop}
        busy
        onAppend={noop}
        labels={LABELS}
      />
    </div>
  )
}
