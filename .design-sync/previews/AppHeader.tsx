import { AppHeader, Button } from '@takomo/web'

const NAV = { board: 'Board', inbox: 'Inbox', initiatives: 'Initiatives', schedules: 'Schedules' }
const NAV_DE = { board: 'Board', inbox: 'Inbox', initiatives: 'Initiativen', schedules: 'Zeitpläne' }
const noop = () => {}
const PROJECTS = [{ id: 'takomo' }, { id: 'demo' }]

/**
 * The header every surface shares. The current surface is a pill rather than a
 * link, so the four pages read as one product instead of four apps that happen
 * to share a palette.
 */
export function Initiatives() {
  return (
    <AppHeader
      current="initiatives"
      nav={NAV}
      lang="en"
      onLang={noop}
      projects={PROJECTS}
      project="takomo"
      allProjectsLabel="All projects"
      onProject={noop}
    >
      <Button>+ New initiative</Button>
      <Button variant="outline" size="icon" title="Refresh">
        ↻
      </Button>
      <Button variant="outline" size="icon" title="Sign out">
        ⎋
      </Button>
    </AppHeader>
  )
}

/** A different surface current, and no project picker — not every page has one. */
export function BoardNoPicker() {
  return <AppHeader current="board" nav={NAV} lang="en" onLang={noop} />
}

/** German, with DE active in the toggle. */
export function German() {
  return (
    <AppHeader
      current="initiatives"
      nav={NAV_DE}
      lang="de"
      onLang={noop}
      projects={PROJECTS}
      project="takomo"
      allProjectsLabel="Alle Projekte"
      projectLabel="Projekt"
      onProject={noop}
    >
      <Button>+ Neue Initiative</Button>
    </AppHeader>
  )
}
