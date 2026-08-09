// Append an entry to an initiative.
//
// Entries are append-only on every surface, so this is the only write here that
// adds to the collection — there is no edit and no delete, by design.
import { Field } from '@/components/Field'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { fmtBytes } from '@/lib/format'
import { KINDS } from '@/lib/initiatives'

export interface Draft {
  kind: string
  source: string
  title: string
  text: string
  uri: string
  origin: string
}

export interface PickedFile {
  name: string
  mime: string
  b64: string
  size: number
}

export interface ComposerProps {
  draft: Draft
  onDraft: (patch: Partial<Draft>) => void
  file: PickedFile | null
  onPickFile: (f: File | null) => void
  busy: boolean
  onAppend: () => void
  labels: {
    kind: string
    kindHint: string
    source: string
    sourceHint: string
    title: string
    titlePh: string
    uri: string
    uriPh: string
    text: string
    textPh: string
    origin: string
    originHint: string
    attach: string
    attachClear: string
    attachAria: string
    append: string
    appending: string
  }
}

export function Composer({
  draft,
  onDraft,
  file,
  onPickFile,
  busy,
  onAppend,
  labels,
}: ComposerProps) {
  return (
    <div className="bg-card border-border rounded-[10px] border px-4 py-3.75">
      <div className="mb-2.25 flex flex-wrap gap-2.5 [&>*]:flex-[1_1_170px]">
        <Field label={labels.kind} hint={labels.kindHint}>
          {(id) => (
            <>
              {/* A datalist, not a select: the kind vocabulary is open by design
                  — a new kind is just a new slug — so the UI suggests without
                  constraining. */}
              <Input
                id={id}
                list="kindlist"
                value={draft.kind}
                onChange={(e) => onDraft({ kind: e.target.value })}
              />
              <datalist id="kindlist">
                {KINDS.map((k) => (
                  <option key={k} value={k} />
                ))}
              </datalist>
            </>
          )}
        </Field>
        <Field label={labels.source} hint={labels.sourceHint}>
          {(id) => (
            <Input
              id={id}
              placeholder="agent:w1"
              value={draft.source}
              onChange={(e) => onDraft({ source: e.target.value })}
            />
          )}
        </Field>
      </div>

      <div className="mb-2.25 flex flex-wrap gap-2.5 [&>*]:flex-[1_1_170px]">
        <Field label={labels.title}>
          {(id) => (
            <Input
              id={id}
              placeholder={labels.titlePh}
              value={draft.title}
              onChange={(e) => onDraft({ title: e.target.value })}
            />
          )}
        </Field>
        <Field label={labels.uri}>
          {(id) => (
            <Input
              id={id}
              placeholder={labels.uriPh}
              value={draft.uri}
              onChange={(e) => onDraft({ uri: e.target.value })}
            />
          )}
        </Field>
      </div>

      <div className="mb-2.25 flex flex-wrap gap-2.5 [&>*]:flex-[1_1_170px]">
        <Field label={labels.text}>
          {(id) => (
            <Textarea
              id={id}
              className="min-h-24"
              placeholder={labels.textPh}
              value={draft.text}
              onChange={(e) => onDraft({ text: e.target.value })}
            />
          )}
        </Field>
      </div>

      <div className="mb-2.25 flex flex-wrap gap-2.5 [&>*]:flex-[1_1_170px]">
        <Field label={labels.origin} hint={labels.originHint}>
          {(id) => (
            <Input
              id={id}
              type="datetime-local"
              value={draft.origin}
              onChange={(e) => onDraft({ origin: e.target.value })}
            />
          )}
        </Field>
      </div>

      <div className="mt-1 flex items-center gap-2.5">
        <div className="text-muted-foreground inline-flex items-center gap-2 text-[12.5px]">
          {file ? (
            <>
              <span className="text-foreground font-mono text-[11.5px]">
                {file.name} · {fmtBytes(file.size)}
              </span>
              <Button variant="outline" size="sm" onClick={() => onPickFile(null)}>
                {labels.attachClear}
              </Button>
            </>
          ) : (
            <>
              <span>{labels.attach}</span>
              <input
                type="file"
        style={{ maxWidth: '100%' }}
                aria-label={labels.attachAria}
                onChange={(e) => onPickFile(e.target.files?.[0] ?? null)}
                className="text-[12.5px]"
              />
            </>
          )}
        </div>
        
        <Button onClick={onAppend} disabled={busy}>
          {busy ? labels.appending : labels.append}
        </Button>
      </div>
    </div>
  )
}
