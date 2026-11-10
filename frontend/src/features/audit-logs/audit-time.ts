/**
 * Absolute audit windows (spec §9.10).
 *
 * A preset is resolved once per user action into an absolute `from`/`to` UTC
 * pair that is written to the URL, so pagination, refreshes and shared links
 * all query the same window instead of a sliding one.
 */

const MINUTE_MS = 60 * 1000
const HOUR_MS = 60 * MINUTE_MS
const DAY_MS = 24 * HOUR_MS

export const AUDIT_MAX_RANGE_MS = 90 * DAY_MS

export const AUDIT_RANGE_IDS = ['15m', '1h', '24h', '7d', '30d'] as const

export type AuditRangePresetId = (typeof AUDIT_RANGE_IDS)[number]

const PRESET_DURATIONS: Record<AuditRangePresetId, number> = {
  '15m': 15 * MINUTE_MS,
  '1h': HOUR_MS,
  '24h': DAY_MS,
  '7d': 7 * DAY_MS,
  '30d': 30 * DAY_MS,
}

export const AUDIT_RANGE_PRESETS = AUDIT_RANGE_IDS.map((id) => ({
  id,
  durationMs: PRESET_DURATIONS[id],
}))

export const AUDIT_DEFAULT_PRESET_ID: AuditRangePresetId = '30d'

export type AuditWindow = {
  from: string
  to: string
  range: AuditRangePresetId | undefined
}

export type AuditWindowSearch = {
  from?: string
  to?: string
  range?: AuditRangePresetId
}

export function isAuditRangePreset(
  value: unknown
): value is AuditRangePresetId {
  return (AUDIT_RANGE_IDS as readonly unknown[]).includes(value)
}

function presetDuration(presetId: AuditRangePresetId): number {
  return PRESET_DURATIONS[presetId]
}

/** Resolve a preset to an absolute window ending at `now`. */
export function presetWindow(
  presetId: AuditRangePresetId,
  now: Date
): { from: string; to: string } {
  return {
    from: new Date(now.getTime() - presetDuration(presetId)).toISOString(),
    to: now.toISOString(),
  }
}

/**
 * Resolve the URL window. Both bounds are always materialized: missing bounds
 * follow the server defaults (`to=now`, `from=to-30d`), and a URL without a
 * window falls back to the last 30 days.
 */
export function resolveAuditWindow(
  search: AuditWindowSearch,
  now: Date = new Date()
): AuditWindow {
  if (search.from && search.to) {
    return { from: search.from, to: search.to, range: search.range }
  }

  if (search.from || search.to) {
    const to = search.to ?? now.toISOString()
    const toMs = Date.parse(to)
    const fallbackFromMs = Number.isNaN(toMs) ? now.getTime() : toMs
    const from =
      search.from ?? new Date(fallbackFromMs - 30 * DAY_MS).toISOString()
    return { from, to, range: undefined }
  }

  const presetId = search.range ?? AUDIT_DEFAULT_PRESET_ID
  return { ...presetWindow(presetId, now), range: presetId }
}

/** Front-end guard for the custom range form; the server stays authoritative. */
export function isValidAuditRange(from: string, to: string): boolean {
  const fromMs = Date.parse(from)
  const toMs = Date.parse(to)
  if (Number.isNaN(fromMs) || Number.isNaN(toMs)) return false
  if (fromMs >= toMs) return false
  return toMs - fromMs <= AUDIT_MAX_RANGE_MS
}
