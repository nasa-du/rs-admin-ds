/**
 * Defense-in-depth masking for audit `details` (spec §9.10).
 *
 * The backend already keeps secrets out of audit rows; the frontend masks an
 * exact key catalog again before rendering or copying. Matching is exact
 * (case-insensitive) on purpose: substring matching would corrupt diagnostic
 * keys such as `permission_keys`, `key_type` and `old_kid`.
 */

export const SENSITIVE_DETAIL_KEYS = [
  'password',
  'password_hash',
  'temporary_password',
  'access_token',
  'refresh_token',
  'authorization',
  'cookie',
  'secret',
  'api_key',
  'private_key',
  'client_secret',
] as const

export const MASKED_VALUE = '***'

const SENSITIVE_KEY_SET = new Set<string>(
  SENSITIVE_DETAIL_KEYS.map((key) => key.toLowerCase())
)

function isSensitiveKey(key: string): boolean {
  return SENSITIVE_KEY_SET.has(key.toLowerCase())
}

/**
 * Return a deep copy with forbidden values replaced by `***`. Non-object
 * values are returned unchanged.
 */
export function maskSensitiveDetails(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map((item) => maskSensitiveDetails(item))
  }

  if (value !== null && typeof value === 'object') {
    const masked: Record<string, unknown> = {}
    for (const [key, child] of Object.entries(value)) {
      masked[key] = isSensitiveKey(key)
        ? MASKED_VALUE
        : maskSensitiveDetails(child)
    }
    return masked
  }

  return value
}

/** Pretty-printed masked JSON for the raw view and the copy action. */
export function maskedDetailsJson(details: unknown): string {
  return JSON.stringify(maskSensitiveDetails(details), null, 2)
}
