import type { ReauthenticateResponse } from '@/api/generated/model'
import { toApiError } from '@/lib/api-error'

/**
 * How long a successful reauthentication absorbs `reauth_required` refusals
 * without prompting again (ADR-0006 §7). Requests that were already in flight
 * when the prompt succeeded still carry the old auth time and need one quiet
 * retry; the server's `auth_sessions.authenticated_at` remains authoritative.
 */
export const REAUTH_RACE_WINDOW_MS = 5_000

/** Rejected to `runWithReauth` callers when the user dismisses the prompt. */
export class ReauthCancelledError extends Error {
  constructor() {
    super('Reauthentication was cancelled.')
    this.name = 'ReauthCancelledError'
  }
}

export type ReauthDialogState = {
  open: boolean
  submitting: boolean
  /** Last failed attempt, rendered inline inside the dialog. */
  error: unknown
}

export type ReauthControllerDeps = {
  /** `POST /auth/reauthenticate`; failures must surface as `ApiError`. */
  reauthenticate: (
    password: string
  ) => Promise<Pick<ReauthenticateResponse, 'access_token'>>
  /** Replace the in-memory access token after a successful prompt. */
  applyAccessToken: (accessToken: string) => void
  /** Clock for the race window; injectable in tests. */
  now?: () => number
}

export type ReauthController = {
  getSnapshot: () => ReauthDialogState
  subscribe: (listener: () => void) => () => void
  /**
   * Open the shared prompt (or join the one already open) and resolve with the
   * new access token. Rejects with `ReauthCancelledError` on dismissal.
   */
  requestReauth: () => Promise<string>
  /** Submit the dialog's current password. Errors stay in `error` state. */
  submitPassword: (password: string) => Promise<void>
  /** Dismiss the prompt and reject its waiters. No-op when already closed. */
  cancel: () => void
  /**
   * Run a reauthentication-gated operation: on 403 `reauth_required`, satisfy
   * one shared prompt and retry `fn()` exactly once.
   */
  runWithReauth: <T>(fn: () => Promise<T>) => Promise<T>
}

const INITIAL_STATE: ReauthDialogState = {
  open: false,
  submitting: false,
  error: null,
}

type PendingReauth = {
  promise: Promise<string>
  resolve: (accessToken: string) => void
  reject: (error: unknown) => void
}

function isReauthRequired(error: unknown): boolean {
  return toApiError(error)?.code === 'reauth_required'
}

export function createReauthController(
  deps: ReauthControllerDeps
): ReauthController {
  const now = deps.now ?? Date.now
  const listeners = new Set<() => void>()
  let state = INITIAL_STATE
  let pending: PendingReauth | null = null
  let lastSuccessAt: number | null = null

  function emit() {
    for (const listener of [...listeners]) listener()
  }

  function setState(next: ReauthDialogState) {
    state = next
    emit()
  }

  function takePending(): PendingReauth | null {
    const current = pending
    pending = null
    return current
  }

  function requestReauth(): Promise<string> {
    if (pending) return pending.promise

    let resolve!: (accessToken: string) => void
    let reject!: (error: unknown) => void
    const promise = new Promise<string>((res, rej) => {
      resolve = res
      reject = rej
    })

    pending = { promise, resolve, reject }
    setState({ open: true, submitting: false, error: null })
    return promise
  }

  function cancel() {
    if (!pending) return
    const current = takePending()
    setState(INITIAL_STATE)
    current?.reject(new ReauthCancelledError())
  }

  function closeWithError(error: unknown) {
    const current = takePending()
    setState(INITIAL_STATE)
    current?.reject(error)
  }

  async function submitPassword(password: string): Promise<void> {
    if (!pending || state.submitting) return
    setState({ ...state, submitting: true, error: null })

    try {
      const response = await deps.reauthenticate(password)
      deps.applyAccessToken(response.access_token)
      lastSuccessAt = now()

      const current = takePending()
      setState(INITIAL_STATE)
      current?.resolve(response.access_token)
    } catch (error) {
      // `cancel()` may have dismissed the prompt while the request was in
      // flight; do not reopen it without a waiter to serve.
      if (!pending) return

      const apiError = toApiError(error)
      if (apiError?.code === 'password_change_required') {
        // The shell listener moves the user to /change-password; the caller
        // must not run the original operation.
        closeWithError(apiError)
        return
      }

      setState({ open: true, submitting: false, error: apiError ?? error })
      throw apiError ?? error
    }
  }

  function recentlyReauthenticated(): boolean {
    return (
      lastSuccessAt !== null && now() - lastSuccessAt < REAUTH_RACE_WINDOW_MS
    )
  }

  async function runWithReauth<T>(fn: () => Promise<T>): Promise<T> {
    try {
      return await fn()
    } catch (error) {
      if (!isReauthRequired(error)) throw error

      // A request that was already in flight when another prompt succeeded
      // still carries the old auth time: retry once without prompting.
      if (recentlyReauthenticated()) return await fn()

      await requestReauth()
      return await fn()
    }
  }

  return {
    getSnapshot: () => state,
    subscribe(listener) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
    requestReauth,
    submitPassword,
    cancel,
    runWithReauth,
  }
}
