import type { Locale } from '@/lib/i18n'

export const projectSections = ['general', 'writing', 'documents', 'workflow'] as const
export type ProjectSection = typeof projectSections[number]
export type SettingsSection = ProjectSection | 'overview' | 'data' | 'access' | 'people' | 'projects' | 'library' | 'search'
export const isProjectSection = (section: string): section is ProjectSection => projectSections.some(key => key === section)
export function settingsSection(search: string): SettingsSection {
  const key = new URLSearchParams(search).get('section')
  return key && (isProjectSection(key) || ['overview', 'data', 'access', 'people', 'projects', 'library', 'search'].includes(key))
    ? key as SettingsSection : 'general'
}
export function settingsHref(section: SettingsSection, project: string) {
  const params = new URLSearchParams({ section })
  if (project) params.set('scope', project)
  return `/settings?${params}`
}
export const settingsLabels = (lang: Locale): Record<SettingsSection, string> => lang === 'de' ? {
  general: 'Allgemein', writing: 'Schreibanweisungen', documents: 'Dokumentdarstellung', workflow: 'Workflow & Laufzeiten',
  overview: 'Deine Sitzung', data: 'Sicherung & Wartung', access: 'API-Tokens', people: 'Personen', projects: 'Projekte', library: 'Workflow-Bibliothek', search: 'Suche',
} : {
  general: 'General', writing: 'Writing instructions', documents: 'Document appearance', workflow: 'Workflow & timing',
  overview: 'Your session', data: 'Backup & maintenance', access: 'API tokens', people: 'People', projects: 'Projects', library: 'Workflow library', search: 'Search',
}
