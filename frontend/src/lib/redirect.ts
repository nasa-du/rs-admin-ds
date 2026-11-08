/**
 * Redirect target sanitization (ADR-0006 §4, spec §9.5).
 *
 * Only in-site, absolute-path targets are accepted. Anything that could leave
 * the origin (`//host`, backslashes, protocols) or loop back to the sign-in
 * page falls back to the dashboard.
 */

const FALLBACK_PATH = '/'
const SITE_ORIGIN = 'http://localhost'
const SIGN_IN_PATHS = new Set(['/sign-in', '/sign-in/'])

function hasForbiddenCharacters(value: string): boolean {
  if (value.includes('\\')) return true
  for (const character of value) {
    const code = character.codePointAt(0) ?? 0
    if (code <= 0x1f || code === 0x7f) return true
  }
  return false
}

export function sanitizeRedirect(raw: unknown): string {
  if (typeof raw !== 'string' || raw === '') return FALLBACK_PATH
  if (!raw.startsWith('/') || raw.startsWith('//')) return FALLBACK_PATH
  if (hasForbiddenCharacters(raw)) return FALLBACK_PATH

  let url: URL
  try {
    url = new URL(raw, SITE_ORIGIN)
  } catch {
    return FALLBACK_PATH
  }

  // `new URL` resolves `//host`/`\\host` to another origin; reject anything
  // that is not the placeholder site.
  if (url.origin !== SITE_ORIGIN) return FALLBACK_PATH
  // Dot segments can normalize a single-slash path into a protocol-relative
  // one (e.g. `/foo/..//evil.example`); the normalized path must stay a
  // single-slash path.
  if (url.pathname.startsWith('//')) return FALLBACK_PATH
  if (SIGN_IN_PATHS.has(url.pathname)) return FALLBACK_PATH

  return `${url.pathname}${url.search}${url.hash}`
}

/**
 * Target used after a successful change-password. A nested
 * `/change-password` redirect would only show the same form again, so it
 * falls back to the dashboard.
 */
export function changePasswordRedirectTarget(raw: unknown): string {
  const target = sanitizeRedirect(raw)
  const pathname = new URL(target, SITE_ORIGIN).pathname.replace(/\/+$/, '')
  return pathname === '/change-password' ? FALLBACK_PATH : target
}
