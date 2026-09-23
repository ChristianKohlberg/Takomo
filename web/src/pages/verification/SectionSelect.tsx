import type { PlanNode } from '@/lib/plan-sections'

/** A native select over the plan's sections; the empty value means no section. */
export function SectionSelect({
  id,
  nodes,
  value,
  noneLabel,
  label,
  onChange,
}: {
  id?: string
  nodes: PlanNode[]
  value: string | null
  noneLabel: string
  label?: string
  onChange: (section: string | null) => void
}) {
  return (
    <select
      id={id}
      aria-label={label}
      value={value ?? ''}
      onChange={(event) => onChange(event.target.value || null)}
      className="bg-card min-w-0 max-w-full rounded-md border px-3 py-2 text-sm"
    >
      <option value="">{noneLabel}</option>
      {nodes.map((node) => (
        <option key={node.id} value={node.id}>
          {node.title}
        </option>
      ))}
      {value && !nodes.some((node) => node.id === value) && <option value={value}>{value}</option>}
    </select>
  )
}
