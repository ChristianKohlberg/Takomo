import { useEffect, useRef } from 'react'
import { Typeahead } from '@takomo/web'

const noop = () => {}
const LABELS = {
  all: 'All tickets',
  placeholder: 'Filter by ticket (id or title)',
  clear: 'Clear filter',
  noMatch: 'No match for “{q}”',
  count: '{n} matches',
  count1: '1 match',
}
const TICKETS = [
  { id: 'demo-2cx4', title: 'Migrate off the billing_v1 table' },
  { id: 'demo-3l2j', title: 'Webhook retries double-charge on 5xx' },
  { id: 'demo-0lj3', title: 'Billing revamp' },
  { id: 'demo-9qbw', title: 'Rate-limit POST /v1/questions per token' },
]

/**
 * ONE component, several mount points — /board's ticket, tag-value, epic and
 * label filters, and /inbox's ticket filter. If they forked, the ARIA and
 * keyboard guarantees would stop covering whichever copy went unmaintained.
 */
export function Empty() {
  return (
    <div style={{ width: 260 }}>
      <Typeahead id="ta-demo-1" options={TICKETS} value="" onChange={noop} labels={LABELS} />
    </div>
  )
}

/** With a selection: the value shows and a clear affordance appears. */
export function Selected() {
  return (
    <div style={{ width: 260 }}>
      <Typeahead id="ta-demo-2" options={TICKETS} value="demo-2cx4" onChange={noop} labels={LABELS} />
    </div>
  )
}

/** Values, not tickets — the same control filtering a tag vocabulary. */
export function TagValues() {
  return (
    <div style={{ width: 260 }}>
      <Typeahead
        id="ta-demo-3"
        options={[{ id: 'component:roadmap' }, { id: 'component:billing' }, { id: 'person:ada' }]}
        value=""
        onChange={noop}
        labels={{ ...LABELS, all: 'All', placeholder: 'Filter by tag value' }}
      />
    </div>
  )
}

/**
 * The state worth seeing: the popup is a `listbox` of `option`s, the input
 * carries `aria-expanded` and `aria-activedescendant`, and ArrowDown/Enter/
 * Escape drive it — none of which is visible while the control is closed. The
 * popup opens on focus, so the preview focuses the input rather than adding a
 * prop that exists only for previews.
 */
export function Open() {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    ref.current?.querySelector('input')?.focus()
  }, [])
  return (
    <div ref={ref} style={{ width: 260, height: 280 }}>
      <Typeahead id="ta-demo-4" options={TICKETS} value="" onChange={noop} labels={LABELS} />
    </div>
  )
}
