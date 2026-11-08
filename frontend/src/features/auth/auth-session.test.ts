import { QueryClient } from '@tanstack/react-query'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { UserSelfResponse } from '@/api/generated/model'
import { ApiError } from '@/lib/api-error'
import {
  authKeys,
  createAuthSession,
  type AuthSessionDeps,
} from './auth-session'
import { authStore } from './auth-store'

const USER: UserSelfResponse = {
  id: 'user-1',
  username: 'alice',
  email: 'alice@example.com',
  display_name: 'Alice',
  status: 'active',
  must_change_password: false,
  is_super_admin: false,
  permissions: [],
  roles: [],
}

function axiosProblem(status: number, code: string, headers = {}) {
  return {
    response: {
      status,
      data: { code, status },
      headers,
    },
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

type SessionOverrides = Partial<Pick<AuthSessionDeps, 'refresh' | 'fetchMe'>>

function setup(overrides: SessionOverrides = {}) {
  const queryClient = new QueryClient()
  const refresh =
    overrides.refresh ?? vi.fn(async () => ({ access_token: 'fresh-token' }))
  const fetchMe = overrides.fetchMe ?? vi.fn(async () => USER)
  const notify = vi.fn<(error: unknown) => void>()

  const session = createAuthSession({
    queryClient,
    refresh,
    fetchMe,
    notify,
  })

  return { session, queryClient, refresh, fetchMe, notify }
}

beforeEach(() => {
  authStore.getState().reset()
})

describe('bootstrap', () => {
  it('refreshes, stores the token, and caches the current user on cold start', async () => {
    const { session, queryClient, refresh, fetchMe } = setup()

    const user = await session.bootstrap()

    expect(refresh).toHaveBeenCalledTimes(1)
    expect(fetchMe).toHaveBeenCalledTimes(1)
    expect(user).toEqual(USER)
    expect(authStore.getState().accessToken).toBe('fresh-token')
    expect(authStore.getState().authStatus).toBe('authenticated')
    expect(queryClient.getQueryData(authKeys.me)).toEqual(USER)
  })

  it('runs refresh then me only once for concurrent bootstrap calls', async () => {
    const refreshResult = deferred<{ access_token: string }>()
    const { session, refresh, fetchMe } = setup({
      refresh: vi.fn(() => refreshResult.promise),
    })

    const first = session.bootstrap()
    const second = session.bootstrap()
    refreshResult.resolve({ access_token: 'fresh-token' })

    await expect(first).resolves.toEqual(USER)
    await expect(second).resolves.toEqual(USER)
    expect(refresh).toHaveBeenCalledTimes(1)
    expect(fetchMe).toHaveBeenCalledTimes(1)
  })

  it('reuses an authenticated token and cached user without refreshing', async () => {
    const { session, queryClient, refresh, fetchMe } = setup()
    authStore.getState().authenticate('existing-token')
    queryClient.setQueryData(authKeys.me, USER)

    await expect(session.bootstrap()).resolves.toEqual(USER)

    expect(refresh).not.toHaveBeenCalled()
    expect(fetchMe).not.toHaveBeenCalled()
  })

  it('silently becomes unauthenticated when refresh returns 401', async () => {
    const { session, queryClient, refresh, notify } = setup({
      refresh: vi.fn(async () => {
        throw axiosProblem(401, 'refresh_token_invalid')
      }),
    })
    queryClient.setQueryData(authKeys.me, USER)

    await expect(session.bootstrap()).resolves.toBeNull()

    expect(authStore.getState().authStatus).toBe('unauthenticated')
    expect(authStore.getState().accessToken).toBeNull()
    expect(queryClient.getQueryCache().getAll()).toHaveLength(0)
    expect(notify).not.toHaveBeenCalled()
    expect(refresh).toHaveBeenCalledTimes(1)
  })

  it('stays unknown and retryable on a network failure', async () => {
    const refresh = vi
      .fn<AuthSessionDeps['refresh']>()
      .mockRejectedValueOnce(new TypeError('Failed to fetch'))
      .mockResolvedValueOnce({ access_token: 'fresh-token' })
    const { session, notify } = setup({ refresh })

    await expect(session.bootstrap()).resolves.toBeNull()
    expect(authStore.getState().authStatus).toBe('unknown')
    expect(authStore.getState().accessToken).toBeNull()
    expect(notify).not.toHaveBeenCalled()

    await expect(session.bootstrap()).resolves.toEqual(USER)
    expect(refresh).toHaveBeenCalledTimes(2)
    expect(authStore.getState().authStatus).toBe('authenticated')
  })

  it('stays unknown and retryable on a 5xx refresh response', async () => {
    const { session, notify } = setup({
      refresh: vi.fn(async () => {
        throw axiosProblem(503, 'service_unavailable')
      }),
    })

    await expect(session.bootstrap()).resolves.toBeNull()

    expect(authStore.getState().authStatus).toBe('unknown')
    expect(notify).not.toHaveBeenCalled()
  })

  it('clears the session and notifies once when the account is disabled', async () => {
    const refresh = vi.fn(async () => {
      throw axiosProblem(403, 'account_disabled')
    })
    const { session, queryClient, notify } = setup({ refresh })
    queryClient.setQueryData(authKeys.me, USER)

    await Promise.all([session.bootstrap(), session.bootstrap()])

    expect(authStore.getState().authStatus).toBe('unauthenticated')
    expect(queryClient.getQueryCache().getAll()).toHaveLength(0)
    expect(notify).toHaveBeenCalledTimes(1)
    expect((notify.mock.calls[0]?.[0] as ApiError).code).toBe(
      'account_disabled'
    )
  })

  it('does not keep a half-restored session when /me fails transiently', async () => {
    const { session, notify } = setup({
      fetchMe: vi.fn(async () => {
        throw new TypeError('Failed to fetch')
      }),
    })

    await expect(session.bootstrap()).resolves.toBeNull()

    expect(authStore.getState().accessToken).toBeNull()
    expect(authStore.getState().authStatus).toBe('unknown')
    expect(notify).not.toHaveBeenCalled()
  })

  it('notifies when /me reports a disabled account', async () => {
    const { session, notify } = setup({
      fetchMe: vi.fn(async () => {
        throw axiosProblem(403, 'account_disabled')
      }),
    })

    await expect(session.bootstrap()).resolves.toBeNull()

    expect(authStore.getState().authStatus).toBe('unauthenticated')
    expect(notify).toHaveBeenCalledTimes(1)
  })
})

describe('refreshAccessToken', () => {
  it('shares one refresh between concurrent callers', async () => {
    const refreshResult = deferred<{ access_token: string }>()
    const { session, refresh } = setup({
      refresh: vi.fn(() => refreshResult.promise),
    })

    const first = session.refreshAccessToken()
    const second = session.refreshAccessToken()
    refreshResult.resolve({ access_token: 'fresh-token' })

    await expect(Promise.all([first, second])).resolves.toEqual([
      'fresh-token',
      'fresh-token',
    ])
    expect(refresh).toHaveBeenCalledTimes(1)
  })

  it('signs out and notifies only once when an authenticated refresh fails', async () => {
    const { session, queryClient, notify } = setup({
      refresh: vi.fn(async () => {
        throw axiosProblem(401, 'refresh_reuse_detected')
      }),
    })
    authStore.getState().authenticate('stale-token')
    queryClient.setQueryData(authKeys.me, USER)

    const first = session.refreshAccessToken()
    const second = session.refreshAccessToken()

    await expect(first).rejects.toBeInstanceOf(ApiError)
    await expect(second).rejects.toBeInstanceOf(ApiError)
    expect(authStore.getState().authStatus).toBe('unauthenticated')
    expect(queryClient.getQueryCache().getAll()).toHaveLength(0)
    expect(notify).toHaveBeenCalledTimes(1)
    expect((notify.mock.calls[0]?.[0] as ApiError).code).toBe('session_expired')
  })

  it('does not sign out when the refresh fails transiently', async () => {
    const { session, notify } = setup({
      refresh: vi.fn(async () => {
        throw new TypeError('Failed to fetch')
      }),
    })
    authStore.getState().authenticate('stale-token')

    await expect(session.refreshAccessToken()).rejects.toThrow(
      'Failed to fetch'
    )

    expect(authStore.getState().accessToken).toBe('stale-token')
    expect(authStore.getState().authStatus).toBe('authenticated')
    expect(notify).not.toHaveBeenCalled()
  })

  it('does not attempt a refresh once the session is unauthenticated', async () => {
    const { session, refresh } = setup()
    authStore.getState().clearSession()

    await expect(session.refreshAccessToken()).rejects.toBeInstanceOf(ApiError)
    expect(refresh).not.toHaveBeenCalled()
  })
})

describe('login and logout transitions', () => {
  it('clears the old query cache before applying a login response', () => {
    const { session, queryClient } = setup()
    queryClient.setQueryData(['users'], [{ id: 'previous-user' }])

    session.applyLogin({ access_token: 'login-token', user: USER })

    expect(queryClient.getQueryData(['users'])).toBeUndefined()
    expect(queryClient.getQueryData(authKeys.me)).toEqual(USER)
    expect(authStore.getState().accessToken).toBe('login-token')
    expect(authStore.getState().authStatus).toBe('authenticated')
  })

  it('clears the store and query cache on logout', () => {
    const { session, queryClient } = setup()
    authStore.getState().authenticate('login-token')
    queryClient.setQueryData(authKeys.me, USER)
    queryClient.setQueryData(['users'], [{ id: 'user-1' }])

    session.signOut()

    expect(authStore.getState().accessToken).toBeNull()
    expect(authStore.getState().authStatus).toBe('unauthenticated')
    expect(queryClient.getQueryCache().getAll()).toHaveLength(0)
  })

  it('clears the session and notifies once on a business account_disabled error', () => {
    const { session, queryClient, notify } = setup()
    authStore.getState().authenticate('login-token')
    queryClient.setQueryData(authKeys.me, USER)
    const error = new ApiError({ code: 'account_disabled', status: 403 })

    session.handleAccountDisabled(error)

    expect(authStore.getState().authStatus).toBe('unauthenticated')
    expect(authStore.getState().accessToken).toBeNull()
    expect(queryClient.getQueryCache().getAll()).toHaveLength(0)
    expect(notify).toHaveBeenCalledTimes(1)
    expect(error.handled).toBe(true)
  })

  it('applies a rotated access token without touching the cached user', () => {
    const { session, queryClient } = setup()
    queryClient.setQueryData(authKeys.me, USER)

    session.applyAccessToken('rotated-token')

    expect(authStore.getState().accessToken).toBe('rotated-token')
    expect(authStore.getState().authStatus).toBe('authenticated')
    expect(queryClient.getQueryData(authKeys.me)).toEqual(USER)
  })
})
