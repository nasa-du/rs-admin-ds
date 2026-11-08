import type { QueryClient } from '@tanstack/react-query'
import { AUTH_ENDPOINTS } from '@/api/auth-endpoints'
import { authHttp } from '@/api/auth-http'
import type {
  LoginResponse,
  RefreshResponse,
  UserSelfResponse,
} from '@/api/generated/model'
import { ApiError, toApiError } from '@/lib/api-error'
import { handleServerError } from '@/lib/handle-server-error'
import { queryClient } from '@/lib/query-client'
import { authStore } from './auth-store'

/** Canonical query key for the current user (ADR-0006 §1). */
export const authKeys = {
  me: ['auth', 'me'] as const,
}

export type AuthSessionDeps = {
  queryClient: Pick<QueryClient, 'getQueryData' | 'setQueryData' | 'clear'>
  refresh: () => Promise<Pick<RefreshResponse, 'access_token'>>
  fetchMe: () => Promise<UserSelfResponse>
  notify: (error: unknown) => void
}

export type AuthSession = {
  /** Cold-start single-flight: refresh, then cache `GET /auth/me`. */
  bootstrap: () => Promise<UserSelfResponse | null>
  /** 401 single-flight: share one refresh between concurrent callers. */
  refreshAccessToken: () => Promise<string>
  /** Business 403 `account_disabled`: clear the session and prompt once. */
  handleAccountDisabled: (error: unknown) => void
  /** Login success: clear old data, then store token and user. */
  applyLogin: (response: Pick<LoginResponse, 'access_token' | 'user'>) => void
  /** Reauth/change-password success: replace the in-memory token. */
  applyAccessToken: (accessToken: string) => void
  /** Logout: clear local state regardless of the server result. */
  signOut: () => void
}

type DefinitiveFailure = 'unauthenticated' | 'account_disabled'

const SESSION_EXPIRED_ERROR = new ApiError({
  code: 'session_expired',
  status: 401,
})

function classifyFailure(error: unknown): DefinitiveFailure | 'transient' {
  const apiError = toApiError(error)
  if (apiError?.code === 'account_disabled') return 'account_disabled'
  if (apiError?.status === 401) return 'unauthenticated'
  return 'transient'
}

export function createAuthSession(deps: AuthSessionDeps): AuthSession {
  let refreshPromise: Promise<string> | null = null
  let bootstrapPromise: Promise<UserSelfResponse | null> | null = null

  function clearSession() {
    authStore.getState().clearSession()
    deps.queryClient.clear()
  }

  /**
   * Tear down the session at most once. Concurrent refresh waiters all observe
   * the same failed promise, but only the first teardown notifies; later
   * failures see `unauthenticated` and stay silent.
   */
  function invalidate(
    kind: DefinitiveFailure,
    error: unknown,
    options: { notify: boolean }
  ) {
    const { authStatus, accessToken } = authStore.getState()
    const alreadySignedOut = authStatus === 'unauthenticated'
    const hadSession = accessToken !== null

    clearSession()

    if (!options.notify) return

    const apiError = toApiError(error) ?? error
    if (!alreadySignedOut) {
      if (kind === 'account_disabled') {
        deps.notify(apiError)
      } else if (hadSession) {
        deps.notify(SESSION_EXPIRED_ERROR)
      }
    }

    // Suppress a second prompt from mutation `onError` for this same failure.
    if (apiError instanceof ApiError) apiError.handled = true
  }

  function refreshAccessToken(): Promise<string> {
    if (authStore.getState().authStatus === 'unauthenticated') {
      return Promise.reject(SESSION_EXPIRED_ERROR)
    }
    if (refreshPromise) return refreshPromise

    const run = Promise.resolve()
      .then(() => deps.refresh())
      .then(
        (response) => {
          authStore.getState().authenticate(response.access_token)
          return response.access_token
        },
        (error: unknown) => {
          const apiError = toApiError(error) ?? error
          const kind = classifyFailure(apiError)
          if (kind !== 'transient') {
            invalidate(kind, apiError, { notify: true })
          }
          throw apiError
        }
      )

    refreshPromise = run.finally(() => {
      refreshPromise = null
    })
    return refreshPromise
  }

  function bootstrap(): Promise<UserSelfResponse | null> {
    const { authStatus } = authStore.getState()
    if (authStatus === 'authenticated') {
      const cached = deps.queryClient.getQueryData<UserSelfResponse>(
        authKeys.me
      )
      if (cached) return Promise.resolve(cached)
    }
    if (bootstrapPromise) return bootstrapPromise

    const run = (async () => {
      try {
        await refreshAccessToken()
      } catch (error) {
        if (classifyFailure(error) === 'transient') {
          // Keep no half-restored session; the guard can retry bootstrap.
          authStore.getState().reset()
        }
        return null
      }

      try {
        const user = await deps.fetchMe()
        deps.queryClient.setQueryData(authKeys.me, user)
        return user
      } catch (error) {
        const apiError = toApiError(error) ?? error
        const kind = classifyFailure(apiError)
        if (kind === 'transient') {
          authStore.getState().reset()
          return null
        }
        invalidate(kind, apiError, { notify: kind === 'account_disabled' })
        return null
      }
    })()

    bootstrapPromise = run.finally(() => {
      bootstrapPromise = null
    })
    return bootstrapPromise
  }

  function handleAccountDisabled(error: unknown) {
    const apiError = toApiError(error)
    if (!apiError) return
    invalidate('account_disabled', apiError, { notify: true })
  }

  function applyLogin(response: Pick<LoginResponse, 'access_token' | 'user'>) {
    deps.queryClient.clear()
    authStore.getState().authenticate(response.access_token)
    deps.queryClient.setQueryData(authKeys.me, response.user)
  }

  function applyAccessToken(accessToken: string) {
    authStore.getState().authenticate(accessToken)
  }

  function signOut() {
    clearSession()
  }

  return {
    bootstrap,
    refreshAccessToken,
    handleAccountDisabled,
    applyLogin,
    applyAccessToken,
    signOut,
  }
}

async function requestRefresh(): Promise<
  Pick<RefreshResponse, 'access_token'>
> {
  const response = await authHttp.post<RefreshResponse>(
    AUTH_ENDPOINTS.refresh,
    {},
    {
      withCredentials: true,
      headers: {
        'Content-Type': 'application/json',
        'X-Requested-With': 'XMLHttpRequest',
      },
    }
  )
  return response.data
}

async function requestMe(): Promise<UserSelfResponse> {
  const accessToken = authStore.getState().accessToken
  const response = await authHttp.get<UserSelfResponse>(AUTH_ENDPOINTS.me, {
    headers: accessToken
      ? { Authorization: `Bearer ${accessToken}` }
      : undefined,
  })
  return response.data
}

/**
 * Application-wide session used by the API client. The injected deps keep the
 * bootstrap/refresh state machine testable without HTTP.
 */
export const authSession = createAuthSession({
  queryClient,
  refresh: requestRefresh,
  fetchMe: requestMe,
  notify: handleServerError,
})
