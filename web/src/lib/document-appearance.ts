import type { CSSProperties } from 'react'

export type DocumentTemplate = 'balanced' | 'strong'
export interface DocumentTypography {
  h1_size: number
  h2_size: number
  h3_size: number
  body_size: number
  heading_weight: number
  line_height: number
  heading_spacing: number
}
export interface DocumentAppearance {
  template: DocumentTemplate
  overrides: Partial<DocumentTypography>
}

export const DOCUMENT_TEMPLATES: Record<DocumentTemplate, Readonly<DocumentTypography>> = {
  balanced: { h1_size: 28, h2_size: 22, h3_size: 18, body_size: 16, heading_weight: 600, line_height: 1.6, heading_spacing: 24 },
  strong: { h1_size: 32, h2_size: 24, h3_size: 20, body_size: 16, heading_weight: 600, line_height: 1.6, heading_spacing: 24 },
}
export const DEFAULT_DOCUMENT_APPEARANCE: DocumentAppearance = { template: 'balanced', overrides: {} }
export const DOCUMENT_APPEARANCE_BOUNDS: Record<keyof DocumentTypography, { min: number; max: number; step: number }> = {
  h1_size: { min: 12, max: 64, step: 1 },
  h2_size: { min: 12, max: 64, step: 1 },
  h3_size: { min: 12, max: 64, step: 1 },
  body_size: { min: 12, max: 24, step: 1 },
  heading_weight: { min: 400, max: 800, step: 100 },
  line_height: { min: 1, max: 2.5, step: 0.1 },
  heading_spacing: { min: 0, max: 48, step: 1 },
}
export const DOCUMENT_APPEARANCE_FIELDS = Object.keys(DOCUMENT_APPEARANCE_BOUNDS) as (keyof DocumentTypography)[]

export function validDocumentValue(key: string, value: unknown): value is number {
  const bounds = Object.hasOwn(DOCUMENT_APPEARANCE_BOUNDS, key) && DOCUMENT_APPEARANCE_BOUNDS[key as keyof DocumentTypography]
  return !!bounds && typeof value === 'number' && Number.isFinite(value) && value >= bounds.min && value <= bounds.max &&
    (key !== 'heading_weight' || value % 100 === 0)
}

export function validDocumentAppearance(config: DocumentAppearance): boolean {
  if (!Object.hasOwn(DOCUMENT_TEMPLATES, config.template)) return false
  return Object.entries(config.overrides).every(([key, value]) => validDocumentValue(key, value))
}

/** Template values with the project's overrides on top. An override that is
 *  not a finite number — a field mid-edit — falls back to the template, so a
 *  preview never receives NaN. */
export function resolveDocumentAppearance(config?: DocumentAppearance | null): DocumentTypography {
  const template = DOCUMENT_TEMPLATES[config?.template ?? 'balanced'] ?? DOCUMENT_TEMPLATES.balanced
  const resolved: DocumentTypography = { ...template }
  for (const key of DOCUMENT_APPEARANCE_FIELDS) {
    const value = config?.overrides[key]
    if (typeof value === 'number' && Number.isFinite(value)) resolved[key] = value
  }
  return resolved
}

export function documentAppearanceStyle(config?: DocumentAppearance | null): CSSProperties {
  const values = resolveDocumentAppearance(config)
  return Object.fromEntries(DOCUMENT_APPEARANCE_FIELDS.map((key) => [
    `--doc-${key.replaceAll('_', '-')}`,
    key === 'heading_weight' || key === 'line_height' ? values[key] : `${values[key]}px`,
  ])) as CSSProperties
}

export function sameDocumentAppearance(a?: DocumentAppearance, b?: DocumentAppearance): boolean {
  a ??= DEFAULT_DOCUMENT_APPEARANCE
  b ??= DEFAULT_DOCUMENT_APPEARANCE
  return a.template === b.template && DOCUMENT_APPEARANCE_FIELDS.every((key) => a.overrides[key] === b.overrides[key])
}
