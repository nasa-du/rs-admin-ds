/** True while `locked_until` is in the future (spec §9.8 status markers). */
export function isAccountLocked(
  lockedUntil: string | null | undefined
): boolean {
  if (!lockedUntil) return false
  const value = new Date(lockedUntil).getTime()
  return !Number.isNaN(value) && value > Date.now()
}
