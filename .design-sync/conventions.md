## Building with Takomo

Takomo is a self-hosted task store that AI agent fleets, orchestrators and humans
all talk to over HTTP. Its surfaces are dense, information-first screens: lists of
work, a reading pane, a composer. Identifiers are monospace, one attribute is
encoded once, and colour carries meaning rather than decoration.

### Setup

There is **no theme provider and no context to wrap**. Import components and use
them. Two exceptions:

- `useToast()` requires a `<ToastProvider>` ancestor. Nothing else does.
- The stylesheet must be loaded (`styles.css`). Every component styles itself
  from it; without it you get unstyled boxes.

Dark mode follows the operating system via `prefers-color-scheme` — there is no
theme class to set and no toggle to build.

### The styling idiom — read this before writing any class

The shipped stylesheet is **pre-compiled**. Tailwind emitted only the utilities
Takomo's own components use, so a class that looks plausible may not exist.
Verified examples: `bg-card`, `bg-background`, `bg-muted`, `bg-secondary`,
`bg-popover`, `text-foreground`, `text-muted-foreground`, `text-primary`,
`text-destructive`, `border-border`, `font-mono` all resolve — and so do the
urgency colours `text-crit`, `text-high`, `bg-low`, `bg-normal`, which the board
and inbox cards use to rank work. What does **not** exist is anything the
components never wrote: `text-warning`, `bg-success`, `text-info`, `gap-7`,
`p-9`, `rounded-3xl`, `text-3xl` and `shadow-2xl` all compile to nothing and
silently do nothing.

That list is not a fixed vocabulary — it is whatever this build's components
happened to use. Re-verify against `_ds_bundle.css` rather than trusting it, and
prefer the components, which never have this problem.

So:

1. **Prefer the components.** They carry the design language already.
2. For your own layout glue, use CSS custom properties, which are all defined in
   the shipped stylesheet and always resolve:
   `--bg`, `--panel`, `--col-bg`, `--text`, `--ink2`, `--muted`, `--border`,
   `--accent`, `--accent2`, `--accent-ink`, `--chip-bg`, `--chip-text`, `--sel`,
   `--hover`, `--shadow`, `--mono`, and the meaning colours `--crit`, `--ok`,
   `--high`, `--normal`, `--low`.

```jsx
<div style={{ background: 'var(--panel)', border: '1px solid var(--border)' }}>
```

Never hardcode a hex value — the palette flips for dark mode and a literal will not.

### Vocabulary that carries meaning

- **Monospace = identifier.** Ticket ids, tokens, tags, actor names (`agent:w1`,
  `ini-tm41jq69`) use `font-mono` or `var(--mono)`. Prose never does.
- `--crit` is error/reject, `--ok` is success, and `--high`/`--normal`/`--low`
  are the urgency scale. Use them for those meanings only.
- Status is a *label*, not a state machine, on initiatives — `StatusBadge` takes
  the already-localized string, it does not translate.

### Localization

Every component that shows text takes it as **props**, already localized — none
of them contain strings. Takomo ships DE and EN, so a screen that hardcodes
English is wrong for half its readers.

### Where the truth is

- `styles.css` and its `@import` closure — the real tokens and utilities.
- `components/<group>/<Name>/<Name>.prompt.md` — per-component usage.
- `components/<group>/<Name>/<Name>.d.ts` — the exact props contract.

### An idiomatic composition

```jsx
<Card>
  <CardHeader>
    <CardTitle>Nested epics on the roadmap</CardTitle>
    <CardDescription>What the roadmap rollup double-counts today.</CardDescription>
    <CardAction><StatusBadge status="parked" label="Parked" /></CardAction>
  </CardHeader>
  <CardContent>
    <span className="font-mono text-muted-foreground">ini-tm41jq69</span>
    <Markdown text={body} />
  </CardContent>
  <CardFooter>
    <Button>Append</Button>
    <Button variant="outline">Cancel</Button>
  </CardFooter>
</Card>
```
