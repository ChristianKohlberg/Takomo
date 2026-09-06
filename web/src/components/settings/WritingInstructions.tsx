import { useEffect, useState } from 'react'
import { Field } from '@/components/Field'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { charCount } from '@/lib/format'
import { defineStrings, type Locale } from '@/lib/i18n'
import {
  getWritingInstructions, saveWritingInstructions,
  TEMPLATE_LIMIT, NAME_LIMIT, INSTRUCTION_LIMIT,
  type WritingInstructions as Settings,
} from '@/lib/writing-instructions'

const STR = defineStrings({
  en: {
    title: 'Writing instructions',
    help: 'Save named instructions for this project. The default guides specification AI actions and is available to agents. Your current request takes precedence.',
    empty: 'No writing instructions yet. Add an instruction in your own words.',
    default: 'Project default', none: 'No default instruction',
    name: 'Name', instruction: 'Instruction', add: 'Add instruction', remove: 'Remove',
    save: 'Save writing instructions', saving: 'Saving…', saved: 'Writing instructions saved.',
    loading: 'Loading writing instructions…', retry: 'Retry',
    invalid: 'Each instruction needs a name (up to 80 characters) and text (up to 4,000 characters).',
    readOnly: 'Writing instructions are read-only for this project.',
    discard: 'Discard changes', unnamed: 'Unnamed instruction',
  },
  de: {
    title: 'Schreibanweisungen',
    help: 'Speichere benannte Anweisungen für dieses Projekt. Der Standard leitet KI-Aktionen in der Spezifikation an und steht Agenten zur Verfügung. Deine aktuelle Anfrage hat Vorrang.',
    empty: 'Noch keine Schreibanweisungen. Formuliere eine Anweisung in deinen eigenen Worten.',
    default: 'Projektstandard', none: 'Keine Standardanweisung',
    name: 'Name', instruction: 'Anweisung', add: 'Anweisung hinzufügen', remove: 'Entfernen',
    save: 'Schreibanweisungen speichern', saving: 'Speichert…', saved: 'Schreibanweisungen gespeichert.',
    loading: 'Schreibanweisungen werden geladen…', retry: 'Erneut versuchen',
    invalid: 'Jede Anweisung braucht einen Namen (bis 80 Zeichen) und einen Text (bis 4.000 Zeichen).',
    readOnly: 'Schreibanweisungen können für dieses Projekt nur gelesen werden.',
    discard: 'Änderungen verwerfen', unnamed: 'Unbenannte Anweisung',
  },
})

// Keying the editor by project and credential prevents a late response from
// one project populating (or saving through) another project's form.
export function WritingInstructions(props: { token: string; project: string; readOnly: boolean; lang: Locale }) {
  return <Editor key={`${props.project}:${props.token}`} {...props} />
}

function Editor({ token, project, readOnly, lang }: { token: string; project: string; readOnly: boolean; lang: Locale }) {
  const t = STR[lang]
  const [value, setValue] = useState<Settings | null>(null)
  const [original, setOriginal] = useState<Settings | null>(null)
  const [error, setError] = useState('')
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)
  const [attempt, setAttempt] = useState(0)
  useEffect(() => {
    const controller = new AbortController()
    getWritingInstructions(token, project, controller.signal).then((settings) => {
      if (controller.signal.aborted) return
      setValue(settings)
      setOriginal(settings)
    }).catch((err: unknown) => {
      if (!controller.signal.aborted) setError(err instanceof Error ? err.message : String(err))
    })
    return () => controller.abort()
  }, [token, project, attempt])

  const dirty = JSON.stringify(value) !== JSON.stringify(original)
  const invalid = value?.templates.some((item) => !item.name.trim() || !item.instruction.trim()
    || charCount(item.name.trim()) > NAME_LIMIT || charCount(item.instruction.trim()) > INSTRUCTION_LIMIT)
  const disabled = readOnly || saving

  function change(next: Settings) {
    setValue(next)
    setSaved(false)
    setError('')
  }

  async function save() {
    if (!value || disabled || invalid || !dirty) return
    setSaving(true)
    setError('')
    setSaved(false)
    try {
      const stored = await saveWritingInstructions(token, project, value)
      setValue(stored)
      setOriginal(stored)
      setSaved(true)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setSaving(false)
    }
  }

  return (
    <section aria-label={t.title} className="flex min-w-0 flex-col gap-4">
      <div>
        <h2 className="text-[15px] font-semibold">{t.title}</h2>
        <p className="text-muted-foreground mt-1 text-[13px]">{t.help}</p>
      </div>
      {!value ? (
        error ? <div role="alert" className="text-destructive text-sm">
          {error}
          <Button variant="secondary" className="ml-2" onClick={() => { setError(''); setAttempt((n) => n + 1) }}>{t.retry}</Button>
        </div> : <p role="status" className="text-muted-foreground text-sm">{t.loading}</p>
      ) : <>
        <Field label={t.default}>
          {(id) => <select id={id} value={value.default_id ?? ''} disabled={disabled}
            className="border-input bg-background h-9 w-full min-w-0 rounded-md border px-3 text-sm"
            onChange={(e) => change({ ...value, default_id: e.target.value || null })}>
            <option value="">{t.none}</option>
            {value.templates.map((item) => <option key={item.id} value={item.id}>{item.name || t.unnamed}</option>)}
          </select>}
        </Field>
        {value.templates.length === 0 && <p className="text-muted-foreground text-sm">{t.empty}</p>}
        {value.templates.map((item, index) => <fieldset key={item.id} disabled={disabled}
          className="border-border-soft flex min-w-0 flex-col gap-3 rounded-lg border p-4">
          <legend className="px-1 text-xs text-muted-foreground">{index + 1}</legend>
          <Field label={t.name}>
            {(id) => <Input id={id} value={item.name} onChange={(e) => change({ ...value,
              templates: value.templates.map((row) => row.id === item.id ? { ...row, name: e.target.value } : row),
            })} />}
          </Field>
          <Field label={t.instruction}>
            {(id) => <Textarea id={id} className="min-h-28" value={item.instruction}
              onChange={(e) => change({ ...value, templates: value.templates.map((row) => row.id === item.id
                ? { ...row, instruction: e.target.value } : row) })} />}
          </Field>
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="text-muted-foreground text-xs tabular-nums">{charCount(item.instruction.trim())} / {INSTRUCTION_LIMIT}</span>
            {!readOnly && <Button variant="ghost" size="sm" onClick={() => change({
              templates: value.templates.filter((row) => row.id !== item.id),
              default_id: value.default_id === item.id ? null : value.default_id,
            })}>{t.remove}</Button>}
          </div>
        </fieldset>)}
        {readOnly ? <p className="text-muted-foreground text-sm">{t.readOnly}</p> : <>
          <div className="flex flex-wrap gap-2">
            <Button variant="secondary" disabled={saving || value.templates.length >= TEMPLATE_LIMIT}
              onClick={() => change({ ...value, templates: [...value.templates,
                { id: crypto.randomUUID(), name: '', instruction: '' }] })}>{t.add}</Button>
            <Button disabled={saving || !dirty || invalid} onClick={() => void save()}>{saving ? t.saving : t.save}</Button>
            {dirty && <Button variant="ghost" disabled={saving} onClick={() => original && change(original)}>{t.discard}</Button>}
          </div>
          {invalid && <p className="text-destructive text-sm">{t.invalid}</p>}
        </>}
        {error && <p role="alert" className="text-destructive text-sm">{error}</p>}
        {saved && <p role="status" className="text-ok text-sm">{t.saved}</p>}
      </>}
    </section>
  )
}
