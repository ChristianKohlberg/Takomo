// The router. Five routes, one bundle, one document.
//
// Every route is still a real URL the server answers — `/board` typed into the
// address bar, an `#a=` answer link mailed to an outside expert, a bookmarked
// `/schedules` all work exactly as before, because the server serves this same
// document on all five paths. What changed is that moving BETWEEN them no
// longer reloads the page.
//
// Most routes are eagerly imported: they share one vendor chunk and splitting
// them would buy nothing. `/documents` and `/mindmaps` are the exceptions and
// have to be, because both are collaborative: they pull in Yjs and its socket,
// and the editor additionally pulls in Tiptap and ProseMirror — together larger
// than the rest of the app, and bytes every other surface would otherwise pay
// for on first paint. `vite.config.ts` splits that dependency into a `collab`
// chunk the two share and an `editor` chunk only one of them needs. Lazy chunks are possible at all because the binary embeds a
// GENERATED asset manifest (build.rs) rather than four names; the note that used
// to stand here, saying a fifth chunk would fail the build, described the older
// contract.
import { lazy, Suspense, useEffect } from 'react'
import { createBrowserRouter, Navigate, Outlet, RouterProvider, useLocation } from 'react-router'
import { App as BoardApp } from './pages/board/App'
import { App as EnvironmentsApp } from './pages/environments/App'
import { App as InboxApp } from './pages/inbox/App'
import { App as InitiativesApp } from './pages/initiatives/App'
import { App as SchedulesApp } from './pages/schedules/App'
import { App as SettingsApp } from './pages/settings/App'
import { LegacySpecificationRedirect } from './pages/specification/LegacyRedirect'
const AgentQueuesApp = lazy(() => import('./pages/agent-queues/App'))
const SpecificationApp = lazy(() => import('./pages/specification/App'))

/** The document title each surface used to carry in its own `<head>`. */
const TITLES: Record<string, string> = {
  '/board': 'takomo · board',
  '/inbox': 'takomo · inbox',
  '/documents': 'takomo · documents',
  '/initiatives': 'takomo · initiatives',
  '/mindmaps': 'takomo · mindmaps',
  '/schedules': 'takomo · schedules',
  '/verification': 'takomo · verification',
  '/environments': 'takomo · environments',
  '/settings': 'takomo · settings',
  '/agent-queues': 'takomo · agent queue',
}

/**
 * Layout route: keeps the tab title in step with the path.
 *
 * With four documents the title came from each `<head>`. With one document it
 * has to be set on navigation, or every surface would still read
 * "takomo · board" after the first hop — and the tab title is how someone finds
 * the inbox again among a dozen open tabs.
 */
function Root() {
  const { pathname } = useLocation()
  useEffect(() => {
    document.title = pathname.endsWith('/specification')
      ? 'takomo · specification'
      : (TITLES[pathname] ?? 'takomo')
  }, [pathname])
  return (
    <Suspense
      fallback={
        <div role="status" className="p-6 text-muted-foreground">
          Loading…
        </div>
      }
    >
      <Outlet />
    </Suspense>
  )
}

const router = createBrowserRouter([
  {
    element: <Root />,
    children: [
      { path: '/board', element: <BoardApp /> },
      { path: '/inbox', element: <InboxApp /> },
      { path: '/documents', element: <LegacySpecificationRedirect /> },
      { path: '/specification', element: <SpecificationApp /> },
      { path: '/projects/:project/specification', element: <SpecificationApp /> },
      { path: '/initiatives', element: <InitiativesApp /> },
      { path: '/mindmaps', element: <LegacySpecificationRedirect /> },
      { path: '/schedules', element: <SchedulesApp /> },
      { path: '/verification', element: <LegacySpecificationRedirect /> },
      { path: '/environments', element: <EnvironmentsApp /> },
      { path: '/settings', element: <SettingsApp /> },
      { path: '/agent-queues', element: <AgentQueuesApp /> },
      // Anything else the server handed this document for. The server serves it
      // only on the five routes, so this is a safety net rather than a real path.
      { path: '*', element: <Navigate to="/board" replace /> },
    ],
  },
])

export function App() {
  return <RouterProvider router={router} />
}
