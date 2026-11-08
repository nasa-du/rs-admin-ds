import type { UserSelfResponse } from '@/api/generated/model'
import { hasPermission } from '@/lib/permissions'
import { sanitizeRedirect } from '@/lib/redirect'
import { requiredPermissionForPath } from '@/lib/route-permissions'
import type { AuthStatus } from './auth-store'

/**
 * Pure route-guard decisions, kept separate from the TanStack route
 * definitions so the ordering rules are unit-testable (spec §9.5).
 */

export type GuardUser = Pick<
  UserSelfResponse,
  'must_change_password' | 'is_super_admin' | 'permissions'
>

export type GuardInput = {
  user: GuardUser | null
  authStatus: AuthStatus
  pathname: string
  href: string
}

export type ProtectedRouteDecision =
  | { kind: 'allow' }
  | { kind: 'sign-in'; redirect: string }
  | { kind: 'change-password'; redirect: string }
  | { kind: 'forbidden'; from: string }
  | { kind: 'bootstrap-error' }

export type SessionRouteDecision =
  | { kind: 'allow' }
  | { kind: 'sign-in'; redirect: string }
  | { kind: 'bootstrap-error' }

function sessionDecision(input: GuardInput): SessionRouteDecision | null {
  if (input.user) return null
  if (input.authStatus === 'unauthenticated') {
    return { kind: 'sign-in', redirect: input.href }
  }
  // Network/5xx during bootstrap: keep the route retryable instead of
  // misreporting a signed-in user as signed out.
  return { kind: 'bootstrap-error' }
}

function isChangePasswordPath(pathname: string): boolean {
  return pathname.replace(/\/+$/, '') === '/change-password'
}

/**
 * `_authenticated` guard order: session → forced password change → route
 * permission (spec §9.5).
 */
export function decideProtectedRoute(
  input: GuardInput
): ProtectedRouteDecision {
  const session = sessionDecision(input)
  if (session) return session

  const user = input.user as GuardUser

  if (user.must_change_password && !isChangePasswordPath(input.pathname)) {
    return { kind: 'change-password', redirect: input.href }
  }

  const required = requiredPermissionForPath(input.pathname)
  if (required && !hasPermission(user, required)) {
    return { kind: 'forbidden', from: input.href }
  }

  return { kind: 'allow' }
}

/**
 * `/change-password` requires a session but skips both the forced-change and
 * permission rules, which is what prevents a redirect loop.
 */
export function decideChangePasswordRoute(
  input: GuardInput
): SessionRouteDecision {
  return sessionDecision(input) ?? { kind: 'allow' }
}

export type SignInRouteDecision =
  { kind: 'allow' } | { kind: 'redirect'; href: string }

/** `/sign-in` only checks whether a session already exists. */
export function decideSignInRoute(input: {
  user: GuardUser | null
  redirect?: unknown
}): SignInRouteDecision {
  if (!input.user) return { kind: 'allow' }
  return { kind: 'redirect', href: sanitizeRedirect(input.redirect) }
}
