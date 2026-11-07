import { useEffect } from 'react'
import { DirectionProvider as RdxDirProvider } from '@radix-ui/react-direction'

/**
 * v1 is LTR-only: both supported locales map to `ltr`.
 *
 * The Radix `DirectionProvider` is kept so RTL support can be re-enabled later
 * without rewriting the component tree.
 */
export function DirectionProvider({ children }: { children: React.ReactNode }) {
  useEffect(() => {
    document.documentElement.setAttribute('dir', 'ltr')
  }, [])

  return <RdxDirProvider dir='ltr'>{children}</RdxDirProvider>
}
