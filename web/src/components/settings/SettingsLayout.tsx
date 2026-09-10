import { useState, type ReactNode } from 'react'
import { Link } from 'react-router'
import { ArrowLeft, Menu } from 'lucide-react'
import type { Locale } from '@/lib/i18n'
import type { Project } from '@/lib/initiatives'
import { Button } from '@/components/ui/button'
import { isProjectSection, projectSections, settingsHref, settingsLabels, type SettingsSection } from './settings-navigation'
import { useDiscardEpoch } from './SettingsDrafts'
import './settings.css'

export function SettingsLayout({ lang, onLang, project, projects, onProject, section, legacy, onSignOut, children }: {
  lang: Locale; onLang: (lang: Locale) => void; project: string; projects: Project[]; onProject: (id: string) => void
  section: SettingsSection; legacy: boolean; onSignOut: () => void; children: ReactNode
}) {
  const [open, setOpen] = useState(false)
  const epoch = useDiscardEpoch()
  const de = lang === 'de'
  const labels = settingsLabels(lang)
  const nav = (keys: readonly SettingsSection[]) => keys.map(key => <Link key={key}
    to={settingsHref(key, project)} aria-current={!legacy && section === key ? 'page' : undefined}
    onClick={() => setOpen(false)} className="settings-link">{labels[key]}</Link>)
  return <div className="settings-surface">
    <header className="settings-header">
      <Link to={project ? `/projects/${encodeURIComponent(project)}/specification` : '/specification'} className="font-heading text-xl font-bold">takomo</Link>
      <div className="flex items-center gap-2">
        <select aria-label={de ? 'Sprache' : 'Language'} value={lang} onChange={e => onLang(e.target.value as Locale)} className="rounded border bg-background p-2 text-xs"><option value="en">EN</option><option value="de">DE</option></select>
        <Button variant="ghost" size="sm" onClick={onSignOut}>{de ? 'Abmelden' : 'Sign out'}</Button>
        <Button variant="ghost" size="icon" className="md:hidden" aria-label={de ? 'Einstellungsnavigation' : 'Settings navigation'} aria-expanded={open} aria-controls="settings-sidebar" onClick={() => setOpen(!open)}><Menu /></Button>
      </div>
    </header>
    <div className="settings-body">
      <aside id="settings-sidebar" className={`settings-sidebar ${open ? 'is-open' : ''}`}>
        <Link className="flex items-center gap-2 text-xs text-muted-foreground" to={project ? `/projects/${encodeURIComponent(project)}/specification` : '/specification'}><ArrowLeft size={14} />{de ? 'Zurück zum Arbeitsbereich' : 'Back to workspace'}</Link>
        <h1 className="text-xl font-semibold">{de ? 'Einstellungen' : 'Settings'}</h1>
        <label className="flex min-w-0 flex-col gap-1 rounded-lg border p-3 text-xs text-muted-foreground">{de ? 'Projekt' : 'Project'}
          <select value={project} onChange={e => onProject(e.target.value)} className="min-w-0 bg-background text-sm font-semibold text-foreground">
            <option value="">{de ? 'Projekt auswählen' : 'Select project'}</option>
            {project && !projects.some(p => p.id === project) && <option value={project}>{project}</option>}
            {projects.map(p => <option key={p.id} value={p.id}>{p.name || p.id}{p.archived ? (de ? ' (archiviert)' : ' (archived)') : ''}</option>)}
          </select>
        </label>
        <nav aria-label={de ? 'Projekteinstellungen' : 'Project settings'}><div className="settings-caption">{de ? 'Projekt' : 'Project settings'}</div>{nav(projectSections)}</nav>
        <nav aria-label={de ? 'Instanzverwaltung' : 'Instance administration'}><div className="settings-caption">{de ? 'Instanzverwaltung' : 'Instance administration'}</div>{nav(['people', 'access', 'projects', 'library', 'github', 'search', 'data'])}</nav>
        <nav className="mt-auto border-t pt-4" aria-label={de ? 'Weitere Bereiche' : 'Other areas'}>
          <Link className="settings-link" to={project ? `/legacy?scope=${encodeURIComponent(project)}` : '/legacy'} aria-current={legacy ? 'page' : undefined} onClick={() => setOpen(false)}>Legacy</Link>
          {nav(['overview'])}
        </nav>
      </aside>
      <main className="settings-main" key={`${legacy}:${section}:${project}:${epoch}`}>
        <div className="settings-content">
          <p className="mb-5 text-xs text-muted-foreground">{legacy ? 'Legacy' : `${de ? 'Einstellungen' : 'Settings'} / ${isProjectSection(section) ? project || (de ? 'Projekt' : 'Project') : section === 'overview' ? (de ? 'Konto' : 'Account') : (de ? 'Instanz' : 'Instance')}`}</p>
          <div className="mb-8 flex flex-wrap items-center justify-between gap-3"><h2 className="text-3xl font-semibold tracking-tight">{legacy ? 'Legacy' : labels[section]}</h2>{!legacy && !isProjectSection(section) && section !== 'overview' && <span className="rounded bg-secondary px-2 py-1 text-xs text-primary">{de ? 'Instanzweit' : 'Instance-wide'}</span>}</div>
          {children}
        </div>
      </main>
    </div>
  </div>
}
