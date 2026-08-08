// The router. Five routes, one bundle, one document.
//
// Every route is still a real URL the server answers — `/board` typed into the
// address bar, an `#a=` answer link mailed to an outside expert, a bookmarked
// `/schedules` all work exactly as before, because the server serves this same
// document on all five paths. What changed is that moving BETWEEN them no
// longer reloads the page.
//
// Routes are eagerly imported on purpose. Lazy routes would emit extra chunks,
// and the binary embeds a fixed set of asset files by name — vite.config.ts
// fails the build if the output ever grows beyond it. Five surfaces sharing one
// vendor chunk is small enough that splitting them buys nothing.
import { useEffect } from 'react'
import { createBrowserRouter, Navigate, Outlet, RouterProvider, useLocation } from 'react-router'
import { App as BoardApp } from './pages/board/App'
import { App as InboxApp } from './pages/inbox/App'
import { App as InitiativesApp } from './pages/initiatives/App'
import { App as SchedulesApp } from './pages/schedules/App'
import { App as SettingsApp } from './pages/settings/App'

/** The document title each surface used to carry in its own `<head>`. */
const TITLES: Record<string, string> = {
  '/board': 'takomo · board',
  '/inbox': 'takomo · inbox',
  '/initiatives': 'takomo · initiatives',
  '/schedules': 'takomo · schedules',
  '/settings': 'takomo · settings',
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
    document.title = TITLES[pathname] ?? 'takomo'
  }, [pathname])
  return <Outlet />
}

const router = createBrowserRouter([
  {
    element: <Root />,
    children: [
      { path: '/board', element: <BoardApp /> },
      { path: '/inbox', element: <InboxApp /> },
      { path: '/initiatives', element: <InitiativesApp /> },
      { path: '/schedules', element: <SchedulesApp /> },
      { path: '/settings', element: <SettingsApp /> },
      // Anything else the server handed this document for. The server serves it
      // only on the five routes, so this is a safety net rather than a real path.
      { path: '*', element: <Navigate to="/board" replace /> },
    ],
  },
])

export function App() {
  return <RouterProvider router={router} />
}
