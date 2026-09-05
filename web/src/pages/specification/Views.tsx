import { Activity, Suspense, useEffect, useState, type ReactNode } from 'react'
import type { SpecificationView } from '@/lib/specification-url'

/** Keep each visited view's DOM and state; pause hidden editors' effects. */
export function SpecificationViews({
  current,
  views,
  loading,
}: {
  current: SpecificationView
  views: Record<SpecificationView, ReactNode>
  loading: string
}) {
  const [visited, setVisited] = useState(() => new Set<SpecificationView>([current]))
  useEffect(() => {
    setVisited((before) => (before.has(current) ? before : new Set([...before, current])))
  }, [current])
  return (
    <>
      {(['document', 'map', 'tests'] as const).map(
        (view) =>
          (visited.has(view) || current === view) && (
            <Activity key={view} mode={current === view ? 'visible' : 'hidden'}>
              <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-specification-view={view}>
                <Suspense
                  fallback={
                    <p role="status" className="p-5 text-sm text-muted-foreground">
                      {loading}
                    </p>
                  }
                >
                  {views[view]}
                </Suspense>
              </div>
            </Activity>
          ),
      )}
    </>
  )
}
