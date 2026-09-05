import { useEffect, useState } from 'react'
import { Navigate, useLocation } from 'react-router'
import { loadProject, loadToken, saveToken, isAuthError } from '@/lib/session'
import { TokenGate } from '@/components/TokenGate'
import { getMindmap } from '@/lib/mindmaps'
import { legacyViews, specificationLink } from '@/lib/specification-url'

/** Resolve a legacy map id before dropping it: it may name another project. */
export function LegacySpecificationRedirect() {
  const location = useLocation()
  const [token, setToken] = useState(loadToken)
  const hash = new URLSearchParams(location.hash.slice(1))
  const legacyMap = hash.get('m')
  const preferred = new URLSearchParams(location.search).get('project') ?? loadProject()
  const [resolved, setResolved] = useState<{ map: string; project: string } | null>(null)
  const [error, setError] = useState('')
  useEffect(() => {
    if (!legacyMap || !token) return
    setError('')
    let cancelled = false
    void getMindmap(token, legacyMap)
      .then((value) => {
        if (!cancelled) setResolved({ map: legacyMap, project: value.mindmap.project })
      })
      .catch((value) => {
        if (!cancelled) {
          if (isAuthError(value)) {
            saveToken('')
            setToken('')
          } else setError(value instanceof Error ? value.message : String(value))
        }
      })
    return () => {
      cancelled = true
    }
  }, [legacyMap, token])
  if (legacyMap && !token)
    return (
      <TokenGate
        title="takomo · specification"
        subtitle="Sign in to open this specification."
        tokenLabel="Token"
        openLabel="Open"
        emptyMessage="Enter a token"
        error=""
        onSubmit={(value) => {
          saveToken(value)
          setToken(value)
        }}
      />
    )
  if (legacyMap && resolved?.map !== legacyMap)
    return (
      <div className="p-6 text-sm" role="status">
        {error || 'Opening specification…'}
        {error && (
          <a className="ml-2 underline" href={specificationLink(preferred)}>
            Specification
          </a>
        )}
      </div>
    )
  const target = new URL(
    specificationLink(
      resolved?.project ?? preferred,
      legacyViews[location.pathname] ?? 'document',
      hash.get('n'),
    ),
    window.location.origin,
  )
  if (hash.get('c')) target.searchParams.set('check', hash.get('c')!)
  return <Navigate to={target.pathname + target.search} replace />
}
