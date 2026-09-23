import type { PlanNode } from '@/lib/plan-sections'
import { cn } from '@/lib/utils'

/** A native select over the plan's sections; the empty value means no section. */
export function SectionSelect({
  id,
  nodes,
  value,
  noneLabel,
  label,
  className,
  onChange,
}: {
  id?: string
  nodes: PlanNode[]
  value: string | null
  noneLabel: string
  label?: string
  className?: string
  onChange: (section: string | null) => void
}) {
  return (
    <select
      id={id}
      aria-label={label}
      value={value ?? ''}
      onChange={(event) => onChange(event.target.value || null)}
      className={cn('bg-card max-w-full min-w-0 rounded-md border px-3 py-2 text-sm', className)}
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
