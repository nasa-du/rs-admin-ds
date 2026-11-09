import { useState } from 'react'

/**
 * A local input value that follows an external source (URL search) until the
 * user types, then re-syncs when the external value changes. Uses the
 * documented render-time state adjustment instead of a setState effect.
 */
export function useSyncedValue(value: string) {
  const [local, setLocal] = useState(value)
  const [synced, setSynced] = useState(value)

  if (value !== synced) {
    setSynced(value)
    setLocal(value)
  }

  return [local, setLocal] as const
}
