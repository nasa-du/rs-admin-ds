import { useEffect, useState } from 'react'
import { getRetryAfterSeconds } from '@/lib/api-error'

/**
 * Countdown for a 429 `Retry-After` hint (spec §9.12). Returns 0 for any
 * non-429 error. The server remains authoritative on the next attempt.
 */
export function useRetryAfter(error: unknown): number {
  const seconds = getRetryAfterSeconds(error)
  const initial = seconds ?? 0
  const [state, setState] = useState({ error, remaining: initial })

  // A new error restarts the countdown even when its value is unchanged.
  if (state.error !== error) {
    setState({ error, remaining: initial })
  }

  useEffect(() => {
    if (initial <= 0) return

    const timer = setInterval(() => {
      setState((previous) => {
        if (previous.remaining <= 1) {
          clearInterval(timer)
          return { ...previous, remaining: 0 }
        }
        return { ...previous, remaining: previous.remaining - 1 }
      })
    }, 1000)

    return () => clearInterval(timer)
  }, [error, initial])

  return state.error === error ? state.remaining : initial
}
