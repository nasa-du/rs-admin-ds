/**
 * Shell-level reactions to API refusals that are not session endings.
 *
 * A restricted session (`must_change_password`) gets 403
 * `password_change_required` from business endpoints; the shell listens and
 * moves the user to `/change-password` (spec §9.5).
 */

type Listener = () => void

const passwordChangeListeners = new Set<Listener>()

export function onPasswordChangeRequired(listener: Listener): () => void {
  passwordChangeListeners.add(listener)
  return () => {
    passwordChangeListeners.delete(listener)
  }
}

/** Called by the API client for a business 403 `password_change_required`. */
export function notifyPasswordChangeRequired(): void {
  for (const listener of [...passwordChangeListeners]) listener()
}

export type PasswordChangeRedirectDeps = {
  pathname: string
  href: string
  redirectToChangePassword: (redirect: string) => void
}

export function handlePasswordChangeRequired(
  deps: PasswordChangeRedirectDeps
): void {
  if (deps.pathname.replace(/\/+$/, '') === '/change-password') return
  deps.redirectToChangePassword(deps.href)
}
