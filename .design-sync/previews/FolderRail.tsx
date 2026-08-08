import { FolderRail } from '@takomo/web'

const LABELS = {
  heading: 'Folders',
  open: 'Open',
  answered: 'Answered',
  withdrawn: 'Withdrawn',
  expired: 'Expired',
}
const FOLDERS = ['open', 'answered', 'withdrawn', 'expired'] as const
const noop = () => {}

const frame: React.CSSProperties = { width: 190, border: '1px solid var(--border)', borderRadius: 10, overflow: 'hidden' }

/** Open is the folder that matters, so its count is the emphatic one. */
export function WithCounts() {
  return (
    <div style={frame}>
      <FolderRail
        folders={FOLDERS}
        current="open"
        counts={{ open: 4, answered: 12, withdrawn: 1 }}
        labels={LABELS}
        onSelect={noop}
      />
    </div>
  )
}

/** All clear: a zero renders nothing at all — a "0" badge is noise. */
export function AllClear() {
  return (
    <div style={frame}>
      <FolderRail folders={FOLDERS} current="open" counts={{ answered: 12 }} labels={LABELS} onSelect={noop} />
    </div>
  )
}

/** Reading an archived folder. */
export function OnAnswered() {
  return (
    <div style={frame}>
      <FolderRail
        folders={FOLDERS}
        current="answered"
        counts={{ open: 2, answered: 12 }}
        labels={LABELS}
        onSelect={noop}
      />
    </div>
  )
}
