import { useCallback, useRef } from 'react'
import { useBlocker } from '@tanstack/react-router'

/**
 * Structural view of TanStack Router's resolver; the package does not export
 * its type. `status` tells the dialog whether a navigation is waiting.
 */
export type UnsavedBlocker = {
  status: 'blocked' | 'idle'
  proceed?: () => void
  reset?: () => void
}

/**
 * In-app and hard-navigation guard for dirty role forms (spec §9.9). While
 * dirty, router navigations are blocked until the user chooses, and
 * `beforeunload` shows the browser's native prompt.
 */
export function useUnsavedChangesGuard(dirty: boolean) {
  const allowNextNavigationRef = useRef(false)

  const shouldBlockFn = useCallback(
    () => dirty && !allowNextNavigationRef.current,
    [dirty]
  )

  const blocker = useBlocker({
    shouldBlockFn,
    enableBeforeUnload: dirty,
    disabled: !dirty,
    withResolver: true,
  }) as UnsavedBlocker | undefined

  /** Let the next router navigation through after a successful save/delete. */
  const allowNextNavigation = useCallback(() => {
    allowNextNavigationRef.current = true
  }, [])

  return {
    blocker: blocker ?? { status: 'idle' as const },
    allowNextNavigation,
  }
}
