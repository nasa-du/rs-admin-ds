/**
 * Auth endpoint paths shared by the API client and the bootstrap transport.
 * Keep endpoint sets here so adding an auth route touches one file.
 */
export const AUTH_ENDPOINTS = {
  login: '/api/v1/auth/login',
  refresh: '/api/v1/auth/refresh',
  logout: '/api/v1/auth/logout',
  logoutAll: '/api/v1/auth/logout-all',
  changePassword: '/api/v1/auth/change-password',
  reauthenticate: '/api/v1/auth/reauthenticate',
  me: '/api/v1/auth/me',
} as const

/** Endpoints that read or rotate the HttpOnly refresh cookie (ADR-0006 §2). */
export const CREDENTIAL_PATHS: ReadonlySet<string> = new Set([
  AUTH_ENDPOINTS.login,
  AUTH_ENDPOINTS.refresh,
  AUTH_ENDPOINTS.logout,
  AUTH_ENDPOINTS.logoutAll,
  AUTH_ENDPOINTS.changePassword,
])

/**
 * Cookie-authenticated POST endpoints that require the CSRF header
 * (ADR-0005 §7.4). `change-password` rotates the cookie but is
 * Bearer-authenticated, so it is not in this set.
 */
export const CSRF_PATHS: ReadonlySet<string> = new Set([
  AUTH_ENDPOINTS.login,
  AUTH_ENDPOINTS.refresh,
  AUTH_ENDPOINTS.logout,
  AUTH_ENDPOINTS.logoutAll,
])

/**
 * Endpoints whose 401 must never start a refresh loop (ADR-0006 §3).
 * `logout-all`, `/me`, and `change-password` may still refresh and retry.
 */
export const NO_REFRESH_PATHS: ReadonlySet<string> = new Set([
  AUTH_ENDPOINTS.login,
  AUTH_ENDPOINTS.refresh,
  AUTH_ENDPOINTS.reauthenticate,
  AUTH_ENDPOINTS.logout,
])
