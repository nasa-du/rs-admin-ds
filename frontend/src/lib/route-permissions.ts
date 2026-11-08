import type { Permission } from './permissions'

/**
 * Central route → permission map (spec §9.2, §9.5). Pages must not repeat
 * these checks; the `_authenticated` guard consumes this table.
 */

type RoutePermissionRule = {
  path: string
  permission: Permission
  exact?: boolean
}

const ROUTE_PERMISSION_RULES: readonly RoutePermissionRule[] = [
  { path: '/roles/new', permission: 'roles.create', exact: true },
  { path: '/users', permission: 'users.read' },
  { path: '/roles', permission: 'roles.read' },
  { path: '/audit-logs', permission: 'audit.read' },
]

const PROTECTED_BASES = ['/users', '/roles', '/audit-logs', '/settings']

function normalizePathname(pathname: string): string {
  if (pathname === '/') return pathname
  return pathname.replace(/\/+$/, '') || '/'
}

function matchesBase(pathname: string, base: string): boolean {
  return pathname === base || pathname.startsWith(`${base}/`)
}

export function requiredPermissionForPath(pathname: string): Permission | null {
  const path = normalizePathname(pathname)

  for (const rule of ROUTE_PERMISSION_RULES) {
    if (rule.exact ? path === rule.path : matchesBase(path, rule.path)) {
      return rule.permission
    }
  }

  return null
}

/**
 * Whether a path renders inside the authenticated shell. Error pages,
 * `/sign-in` and unknown paths are public; `/change-password` requires a
 * session even though it does not use the sidebar layout.
 */
export function isProtectedPath(pathname: string): boolean {
  const path = normalizePathname(pathname)
  if (path === '/' || path === '/change-password') return true
  return PROTECTED_BASES.some((base) => matchesBase(path, base))
}
