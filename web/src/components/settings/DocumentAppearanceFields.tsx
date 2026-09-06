import { Field } from '@/components/Field'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  DEFAULT_DOCUMENT_APPEARANCE, DOCUMENT_APPEARANCE_BOUNDS, DOCUMENT_APPEARANCE_FIELDS,
  resolveDocumentAppearance, type DocumentAppearance, type DocumentTemplate, type DocumentTypography,
} from '@/lib/document-appearance'

export interface DocumentAppearanceLabels {
  title: string
  help: string
  template: string
  balanced: string
  strong: string
  reset: string
  preview: string
  h1: string
  h2: string
  h3: string
  body: string
  fields: Record<keyof DocumentTypography, string>
}

export function DocumentAppearanceFields({ value = DEFAULT_DOCUMENT_APPEARANCE, onChange, disabled, labels }: {
  value?: DocumentAppearance
  onChange: (value: DocumentAppearance) => void
  disabled: boolean
  labels: DocumentAppearanceLabels
}) {
  const effective = resolveDocumentAppearance(value)
  const update = (key: keyof DocumentTypography, number?: number) => {
    const overrides = { ...value.overrides }
    if (number === undefined) delete overrides[key]
    else overrides[key] = number
    onChange({ ...value, overrides })
  }
  return (
    <section className="flex min-w-0 flex-col gap-3" aria-label={labels.title}>
      <div>
        <h2 className="text-[15px] font-semibold">{labels.title}</h2>
        <p className="text-muted-foreground text-[13px]">{labels.help}</p>
      </div>
      <Field label={labels.template}>
        {(id) => <select id={id} value={value.template} disabled={disabled}
          className="border-input bg-background h-9 w-full rounded-md border px-3 text-sm disabled:opacity-50"
          onChange={(event) => onChange({ ...value, template: event.target.value as DocumentTemplate })}>
          <option value="balanced">{labels.balanced} · 28 / 22 / 18</option>
          <option value="strong">{labels.strong} · 32 / 24 / 20</option>
        </select>}
      </Field>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {DOCUMENT_APPEARANCE_FIELDS.map((key) => {
          const overridden = value.overrides[key] !== undefined
          const bounds = DOCUMENT_APPEARANCE_BOUNDS[key]
          const invalid = !Number.isFinite(effective[key]) || effective[key] < bounds.min || effective[key] > bounds.max ||
            (key === 'heading_weight' && effective[key] % 100 !== 0)
          return <Field key={key} label={labels.fields[key]}>
            {(id) => <div className="flex min-w-0 items-center gap-1">
              <Input id={id} type="number" {...bounds} disabled={disabled} aria-invalid={invalid || undefined}
                value={effective[key]} onChange={(event) => update(key, event.target.value === '' ? undefined : Number(event.target.value))} />
              {overridden && <Button type="button" variant="ghost" size="sm" disabled={disabled}
                aria-label={`${labels.reset}: ${labels.fields[key]}`} onClick={() => update(key)}>{labels.reset}</Button>}
            </div>}
          </Field>
        })}
      </div>
      <div className="border-border-soft bg-card min-w-0 rounded-lg border p-5" aria-label={labels.preview}>
        <div className="text-muted-foreground mb-3 text-xs">{labels.preview}</div>
        <div className="break-words" style={{ fontSize: effective.body_size, lineHeight: effective.line_height }}>
          {(['h1', 'h2', 'h3'] as const).map((level, index) => <div key={level}>
            <div style={{ fontSize: effective[`${level}_size`], fontWeight: effective.heading_weight, fontFamily: 'var(--font-heading)', lineHeight: 1.25,
              marginTop: index ? effective.heading_spacing : 0, marginBottom: 8 }}>{labels[level]}</div>
            {index === 2 && <p>{labels.body}</p>}
          </div>)}
        </div>
      </div>
    </section>
  )
}
