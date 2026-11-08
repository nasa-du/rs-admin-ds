import {
  AxiosError,
  type AxiosAdapter,
  type AxiosResponse,
  type InternalAxiosRequestConfig,
} from 'axios'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ApiError } from '@/lib/api-error'
import { handleServerError } from '@/lib/handle-server-error'
import { queryClient } from '@/lib/query-client'
import { authStore } from '@/features/auth/auth-store'
import { authHttp } from './auth-http'
import { apiClient, customInstance } from './mutator'

const toastError = vi.hoisted(() => vi.fn())

vi.mock('sonner', () => ({
  toast: {
    error: toastError,
  },
}))

const originalAdapter = apiClient.defaults.adapter

function useAdapter(adapter: AxiosAdapter) {
  apiClient.defaults.adapter = adapter
}

function okResponse(
  config: InternalAxiosRequestConfig,
  data: unknown
): AxiosResponse {
  return {
    data,
    status: 200,
    statusText: 'OK',
    headers: {},
    config,
  }
}

function problemResponse(
  config: InternalAxiosRequestConfig,
  status: number,
  data: unknown,
  headers: Record<string, string> = {}
) {
  const response: AxiosResponse = {
    data,
    status,
    statusText: '',
    headers,
    config,
  }

  return new AxiosError(
    `Request failed with status ${status}`,
    'ERR_BAD_RESPONSE',
    config,
    {},
    response
  )
}

beforeEach(() => {
  authStore.getState().reset()
  queryClient.clear()
  toastError.mockClear()
})

afterEach(() => {
  apiClient.defaults.adapter = originalAdapter
  vi.restoreAllMocks()
})

describe('request headers', () => {
  it('adds the in-memory bearer token to requests', async () => {
    authStore.getState().authenticate('token-123')
    let seen: InternalAxiosRequestConfig | undefined

    useAdapter(async (config) => {
      seen = config
      return okResponse(config, { ok: true })
    })

    await customInstance({ url: '/api/v1/users', method: 'GET' })

    expect(seen?.headers.get('Authorization')).toBe('Bearer token-123')
  })

  it('sends credentials on cookie-auth endpoints and CSRF only where required', async () => {
    const seen: InternalAxiosRequestConfig[] = []

    useAdapter(async (config) => {
      seen.push(config)
      return okResponse(config, {})
    })

    for (const path of [
      '/api/v1/auth/login',
      '/api/v1/auth/refresh',
      '/api/v1/auth/logout',
      '/api/v1/auth/logout-all',
      '/api/v1/auth/change-password',
      '/api/v1/auth/reauthenticate',
    ]) {
      await customInstance({ url: path, method: 'POST', data: {} })
    }
    await customInstance({ url: '/api/v1/users', method: 'GET' })

    const [login, refresh, logout, logoutAll, changePassword, reauth, users] =
      seen

    for (const config of [login, refresh, logout, logoutAll]) {
      expect(config.withCredentials).toBe(true)
      expect(config.headers.get('X-Requested-With')).toBe('XMLHttpRequest')
      expect(config.headers.get('Content-Type')).toContain('application/json')
    }

    // change-password rotates the refresh cookie but is Bearer-authenticated
    // and does not require CSRF (ADR-0005 §7.4).
    expect(changePassword.withCredentials).toBe(true)
    expect(changePassword.headers.get('X-Requested-With')).toBeFalsy()

    for (const config of [reauth, users]) {
      expect(config.withCredentials).toBeFalsy()
      expect(config.headers.get('X-Requested-With')).toBeFalsy()
    }
  })

  it('refreshes and retries a request once after a 401', async () => {
    authStore.getState().authenticate('stale-token')
    const refresh = vi.spyOn(authHttp, 'post').mockResolvedValue({
      data: {
        access_token: 'fresh-token',
        expires_in: 900,
        token_type: 'Bearer',
      },
    } as never)
    const authorizations: Array<string | null | undefined> = []

    useAdapter(async (config) => {
      const authorization = config.headers.get('Authorization')
      authorizations.push(
        typeof authorization === 'string' ? authorization : undefined
      )

      if (authorization === 'Bearer fresh-token') {
        return okResponse(config, { ok: true })
      }
      throw problemResponse(config, 401, {
        code: 'access_token_invalid',
        status: 401,
      })
    })

    await expect(
      customInstance({ url: '/api/v1/users', method: 'GET' })
    ).resolves.toEqual({ ok: true })

    expect(refresh).toHaveBeenCalledTimes(1)
    expect(authorizations).toEqual(['Bearer stale-token', 'Bearer fresh-token'])
  })

  it('shares one refresh between concurrent 401 responses', async () => {
    authStore.getState().authenticate('stale-token')
    const refresh = vi.spyOn(authHttp, 'post').mockResolvedValue({
      data: {
        access_token: 'fresh-token',
        expires_in: 900,
        token_type: 'Bearer',
      },
    } as never)
    let attempts = 0

    useAdapter(async (config) => {
      attempts += 1
      if (config.headers.get('Authorization') === 'Bearer fresh-token') {
        return okResponse(config, { ok: true })
      }
      throw problemResponse(config, 401, {
        code: 'access_token_invalid',
        status: 401,
      })
    })

    await expect(
      Promise.all([
        customInstance({ url: '/api/v1/users', method: 'GET' }),
        customInstance({ url: '/api/v1/roles', method: 'GET' }),
      ])
    ).resolves.toEqual([{ ok: true }, { ok: true }])

    expect(refresh).toHaveBeenCalledTimes(1)
    expect(attempts).toBe(4)
  })

  it('retries a late 401 with the token from an already-completed refresh', async () => {
    authStore.getState().authenticate('stale-token')
    const refresh = vi.spyOn(authHttp, 'post').mockResolvedValue({
      data: {
        access_token: 'fresh-token',
        expires_in: 900,
        token_type: 'Bearer',
      },
    } as never)
    let releaseLate401: () => void = () => undefined
    const late401 = new Promise<void>((resolve) => {
      releaseLate401 = resolve
    })

    useAdapter(async (config) => {
      const authorization = config.headers.get('Authorization')
      if (authorization === 'Bearer fresh-token') {
        return okResponse(config, { ok: true })
      }
      if (config.url === '/api/v1/users') {
        throw problemResponse(config, 401, {
          code: 'access_token_invalid',
          status: 401,
        })
      }
      await late401
      throw problemResponse(config, 401, {
        code: 'access_token_invalid',
        status: 401,
      })
    })

    const first = customInstance({ url: '/api/v1/users', method: 'GET' })
    const second = customInstance({ url: '/api/v1/roles', method: 'GET' })

    await vi.waitFor(() => expect(refresh).toHaveBeenCalledTimes(1))
    releaseLate401()

    await expect(Promise.all([first, second])).resolves.toEqual([
      { ok: true },
      { ok: true },
    ])
    expect(refresh).toHaveBeenCalledTimes(1)
  })

  it('does not retry a request more than once', async () => {
    authStore.getState().authenticate('stale-token')
    const refresh = vi.spyOn(authHttp, 'post').mockResolvedValue({
      data: {
        access_token: 'fresh-token',
        expires_in: 900,
        token_type: 'Bearer',
      },
    } as never)
    let attempts = 0

    useAdapter(async (config) => {
      attempts += 1
      throw problemResponse(config, 401, {
        code: 'access_token_invalid',
        status: 401,
      })
    })

    const error = await customInstance({
      url: '/api/v1/users',
      method: 'GET',
    }).catch((caught: unknown) => caught)

    expect(error).toBeInstanceOf(ApiError)
    expect(attempts).toBe(2)
    expect(refresh).toHaveBeenCalledTimes(1)
  })

  it('does not refresh on a failed login', async () => {
    const refresh = vi.spyOn(authHttp, 'post')

    useAdapter(async (config) => {
      throw problemResponse(config, 401, {
        code: 'invalid_credentials',
        status: 401,
      })
    })

    const error = await customInstance({
      url: '/api/v1/auth/login',
      method: 'POST',
      data: { identifier: 'alice', password: 'wrong' },
    }).catch((caught: unknown) => caught)

    expect(error).toBeInstanceOf(ApiError)
    expect((error as ApiError).code).toBe('invalid_credentials')
    expect(refresh).not.toHaveBeenCalled()
  })

  it('signs out once when the shared refresh fails', async () => {
    authStore.getState().authenticate('stale-token')
    queryClient.setQueryData(['users'], [{ id: 'user-1' }])
    vi.spyOn(authHttp, 'post').mockRejectedValue(
      problemResponse({} as InternalAxiosRequestConfig, 401, {
        code: 'refresh_reuse_detected',
        status: 401,
      })
    )

    useAdapter(async (config) => {
      throw problemResponse(config, 401, {
        code: 'access_token_invalid',
        status: 401,
      })
    })

    const [first, second] = await Promise.all([
      customInstance({ url: '/api/v1/users', method: 'GET' }).catch(
        (caught: unknown) => caught
      ),
      customInstance({ url: '/api/v1/roles', method: 'GET' }).catch(
        (caught: unknown) => caught
      ),
    ])

    expect(first).toBeInstanceOf(ApiError)
    expect(second).toBeInstanceOf(ApiError)
    expect(authStore.getState().authStatus).toBe('unauthenticated')
    expect(authStore.getState().accessToken).toBeNull()
    expect(queryClient.getQueryCache().getAll()).toHaveLength(0)
    expect(toastError).toHaveBeenCalledTimes(1)
  })

  it('clears the session and prompts once on a business 403 account_disabled', async () => {
    authStore.getState().authenticate('stale-token')
    queryClient.setQueryData(['users'], [{ id: 'user-1' }])

    useAdapter(async (config) => {
      throw problemResponse(config, 403, {
        code: 'account_disabled',
        status: 403,
      })
    })

    const error = await customInstance({
      url: '/api/v1/users',
      method: 'GET',
    }).catch((caught: unknown) => caught)

    expect(error).toBeInstanceOf(ApiError)
    expect(authStore.getState().authStatus).toBe('unauthenticated')
    expect(authStore.getState().accessToken).toBeNull()
    expect(queryClient.getQueryCache().getAll()).toHaveLength(0)
    expect(toastError).toHaveBeenCalledTimes(1)

    // A mutation's onError must not repeat the prompt.
    handleServerError(error)
    expect(toastError).toHaveBeenCalledTimes(1)
  })

  it('keeps the session when the refresh fails transiently', async () => {
    authStore.getState().authenticate('stale-token')
    vi.spyOn(authHttp, 'post').mockRejectedValue(
      new AxiosError('Network Error', 'ERR_NETWORK')
    )

    useAdapter(async (config) => {
      throw problemResponse(config, 401, {
        code: 'session_invalid',
        status: 401,
      })
    })

    const error = await customInstance({
      url: '/api/v1/users',
      method: 'GET',
    }).catch((caught: unknown) => caught)

    expect(error).toBeInstanceOf(ApiError)
    expect(authStore.getState().accessToken).toBe('stale-token')
    expect(authStore.getState().authStatus).toBe('authenticated')
    expect(toastError).not.toHaveBeenCalled()
  })
})

describe('error normalization', () => {
  it('preserves ProblemDetails fields and Retry-After', async () => {
    useAdapter(async (config) => {
      throw problemResponse(
        config,
        429,
        {
          code: 'rate_limited',
          status: 429,
          request_id: 'req-1',
          errors: [{ pointer: '/identifier', code: 'required' }],
        },
        { 'Retry-After': '17' }
      )
    })

    const error = await customInstance({
      url: '/api/v1/users',
      method: 'GET',
    }).catch((caught: unknown) => caught)

    expect(error).toBeInstanceOf(ApiError)
    const apiError = error as ApiError
    expect(apiError.status).toBe(429)
    expect(apiError.code).toBe('rate_limited')
    expect(apiError.requestId).toBe('req-1')
    expect(apiError.fieldErrors).toEqual([
      { pointer: '/identifier', code: 'required' },
    ])
    expect(apiError.retryAfterSeconds).toBe(17)
  })

  it('falls back to a status-code error for a non-ProblemDetails response', async () => {
    useAdapter(async (config) => {
      throw problemResponse(config, 502, '<html>bad gateway</html>', {
        'x-request-id': 'req-gateway',
      })
    })

    const error = await customInstance({
      url: '/api/v1/users',
      method: 'GET',
    }).catch((caught: unknown) => caught)

    expect(error).toBeInstanceOf(ApiError)
    const apiError = error as ApiError
    expect(apiError.status).toBe(502)
    expect(apiError.code).toBe('service_unavailable')
    expect(apiError.requestId).toBe('req-gateway')
  })

  it('normalizes network failures to a network_error ApiError', async () => {
    useAdapter(async (config) => {
      throw new AxiosError('Network Error', 'ERR_NETWORK', config)
    })

    const error = await customInstance({
      url: '/api/v1/users',
      method: 'GET',
    }).catch((caught: unknown) => caught)

    expect(error).toBeInstanceOf(ApiError)
    expect((error as ApiError).code).toBe('network_error')
  })
})
