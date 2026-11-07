/**
 * Cookie utility functions using manual document.cookie approach
 * Replaces js-cookie dependency for better consistency
 */

const DEFAULT_MAX_AGE = 60 * 60 * 24 * 7 // 7 days

export type CookieOptions = {
  maxAge?: number
  sameSite?: 'Lax' | 'Strict' | 'None'
  secure?: boolean
  path?: string
}

/**
 * Get a cookie value by name
 */
export function getCookie(name: string): string | undefined {
  if (typeof document === 'undefined') return undefined

  const value = `; ${document.cookie}`
  const parts = value.split(`; ${name}=`)
  if (parts.length === 2) {
    const cookieValue = parts.pop()?.split(';').shift()
    return cookieValue
  }
  return undefined
}

/**
 * Set a cookie with name, value, and optional attributes.
 *
 * Passing a number keeps the legacy shorthand for `maxAge`.
 */
export function setCookie(
  name: string,
  value: string,
  options: number | CookieOptions = DEFAULT_MAX_AGE
): void {
  if (typeof document === 'undefined') return

  const {
    maxAge = DEFAULT_MAX_AGE,
    sameSite,
    secure,
    path = '/',
  } = typeof options === 'number' ? { maxAge: options } : options

  const attributes = [`path=${path}`, `max-age=${maxAge}`]
  if (sameSite) attributes.push(`SameSite=${sameSite}`)
  if (secure) attributes.push('Secure')

  document.cookie = `${name}=${value}; ${attributes.join('; ')}`
}

/**
 * Remove a cookie by setting its max age to 0
 */
export function removeCookie(name: string): void {
  if (typeof document === 'undefined') return

  document.cookie = `${name}=; path=/; max-age=0`
}
