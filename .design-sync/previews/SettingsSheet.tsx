import { SettingsSheet } from '@takomo/web'

const noop = () => {}
const L = {
  title: 'Project settings',
  subtitle: 'How this project writes, how long an answer link stays valid, and how long an agent holds a ticket.',
  langLabel: 'Question language',
  langHelp: 'The human-facing language agents should phrase ask-a-human questions in. Leave empty for no preference.',
  langPh: 'German',
  styleLabel: 'Style guide',
  styleHelp: 'The house style for text agents write. Agents get it on every work-loop call, before they write.',
  stylePh: 'Terse. No marketing voice.',
  ttlLabel: 'Answer-link lifetime',
  ttlHelp: 'How long a link handed to someone outside the org stays valid. Unlike the two fields above, this one is enforced.',
  claimTtlLabel: 'Default claim lifetime',
  claimTtlHelp: 'How long an agent holds a ticket when it asks for no lifetime of its own.',
  maxClaimTtlLabel: 'Maximum claim lifetime',
  maxClaimTtlHelp: 'The ceiling an agent may ask for — also how long a crashed agent blocks a ticket.',
  chars: '{n} / {max}',
  over: 'The style guide is over the limit.',
  save: 'Save', saving: 'Saving…', savedMsg: 'Saved.', cancel: 'Cancel',
  readOnlyMsg: 'This token cannot change project settings.',
}
const S = { language: 'German', style: 'Terse. Say what changed and why.', ttl: '604800', claimTtl: '900', maxClaimTtl: '3600' }

/** Filled in, ready to save. */
export function Filled() {
  return (
    <SettingsSheet
      open onOpenChange={noop} settings={S} onChange={noop}
      readOnly={false} saving={false} saved={false} onSave={noop} labels={L}
    />
  )
}

/**
 * Read-only: everything is shown, nothing can be saved. Save stays focusable and
 * says why rather than going silently inert.
 */
export function ReadOnly() {
  return (
    <SettingsSheet
      open onOpenChange={noop} settings={S} onChange={noop}
      readOnly saving={false} saved={false} onSave={noop} labels={L}
    />
  )
}
