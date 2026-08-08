import { Field, Input, Textarea } from '@takomo/web'

/**
 * Field wires the label to its control with a generated id, so a caller never
 * has to invent one. It takes a render function and hands it that id.
 */
export function WithHint() {
  return (
    <div style={{ maxWidth: 420 }}>
      <Field
        label="Source"
        hint="Required — where did this come from? agent:w1, person:ada, claude:chat"
      >
        {(id) => <Input id={id} defaultValue="claude:chat" />}
      </Field>
    </div>
  )
}

/** Without a hint — the majority case. */
export function Plain() {
  return (
    <div style={{ maxWidth: 420 }}>
      <Field label="Heading">{(id) => <Input id={id} placeholder="optional" />}</Field>
    </div>
  )
}

/** Two fields side by side, which is how the composer lays out a row. */
export function Row() {
  return (
    <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap', maxWidth: 560 }}>
      <Field label="Kind" hint="note, research, feedback, transcript, document …">
        {(id) => <Input id={id} defaultValue="research" />}
      </Field>
      <Field label="Link">
        {(id) => <Input id={id} placeholder="https://… (optional)" />}
      </Field>
    </div>
  )
}

/** Any control works — the field only owns the label wiring. */
export function WithTextarea() {
  return (
    <div style={{ maxWidth: 480 }}>
      <Field label="Text">
        {(id) => (
          <Textarea id={id} className="min-h-20" placeholder="Markdown. A note, a finding, feedback …" />
        )}
      </Field>
    </div>
  )
}
