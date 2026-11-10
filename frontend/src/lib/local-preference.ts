/**
 * Browser-only preference storage (spec §9.11). Appearance, Notifications and
 * Display never touch the API; they persist in this browser only.
 */

function storage(): Storage | null {
  if (typeof window === 'undefined') return null
  try {
    return window.localStorage
  } catch {
    return null
  }
}

export function readLocalPreference<T>(key: string, fallback: T): T {
  const store = storage()
  if (!store) return fallback

  try {
    const raw = store.getItem(key)
    return raw === null ? fallback : (JSON.parse(raw) as T)
  } catch {
    return fallback
  }
}

export function writeLocalPreference(key: string, value: unknown): void {
  try {
    storage()?.setItem(key, JSON.stringify(value))
  } catch {
    // Quota or privacy-mode failures must never block the form.
  }
}
