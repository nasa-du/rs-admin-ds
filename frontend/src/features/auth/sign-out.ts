import i18n from '@/i18n'
import { toast } from 'sonner'
import { logout } from '@/api/generated/rs-admin'
import { isProtectedPath } from '@/lib/route-permissions'
import { broadcastSignedOut } from './auth-broadcast'
import { authSession } from './auth-session'

/**
 * Sign-out and cross-tab sign-out behavior (spec §9.6).
 *
 * Local state must be cleared whether or not the server request succeeds; a
 * failure only changes the message the user sees.
 */

export type PerformSignOutDeps = {
  requestLogout: () => Promise<unknown>
  signOut: () => void
  broadcast: () => void
  notifyServerFailure: () => void
}

export async function performSignOut(
  deps: PerformSignOutDeps
): Promise<{ ok: boolean }> {
  let ok = true
  try {
    await deps.requestLogout()
  } catch {
    ok = false
  }

  deps.signOut()
  deps.broadcast()
  if (!ok) deps.notifyServerFailure()

  return { ok }
}

export type RemoteSignOutDeps = {
  signOut: () => void
  pathname: string
  href: string
  redirectToSignIn: (redirect: string) => void
}

/** Another tab signed out: clear local state; protected pages go to sign-in. */
export function handleRemoteSignOut(deps: RemoteSignOutDeps): void {
  deps.signOut()
  if (isProtectedPath(deps.pathname)) deps.redirectToSignIn(deps.href)
}

/** Application-wide sign-out used by the confirmation dialog. */
export function signOutOfApp(): Promise<{ ok: boolean }> {
  return performSignOut({
    requestLogout: () => logout({}),
    signOut: () => authSession.signOut(),
    broadcast: () => broadcastSignedOut(),
    notifyServerFailure: () => {
      toast.error(i18n.t('auth:signOut.serverFailed'))
    },
  })
}
