import axios, {
  type AxiosRequestConfig,
  type InternalAxiosRequestConfig,
} from 'axios'
import { type ApiError, toApiError } from '@/lib/api-error'
import { authSession } from '@/features/auth/auth-session'
import { authStore } from '@/features/auth/auth-store'
import {
  CREDENTIAL_PATHS,
  CSRF_PATHS,
  NO_REFRESH_PATHS,
} from './auth-endpoints'
import { apiBaseUrl } from './base-url'

const REFRESHABLE_CODES = new Set(['access_token_invalid', 'session_invalid'])

type RetryableConfig = InternalAxiosRequestConfig & {
  _authRetry?: boolean
}

export const apiClient = axios.create({
  baseURL: apiBaseUrl,
  headers: { Accept: 'application/json' },
})

function pathnameOf(url: string | undefined): string {
  if (!url) return ''
  try {
    return new URL(url, 'http://localhost').pathname
  } catch {
    return url.split('?')[0] ?? url
  }
}

apiClient.interceptors.request.use((config) => {
  const path = pathnameOf(config.url)
  const accessToken = authStore.getState().accessToken

  if (accessToken) {
    config.headers.set('Authorization', `Bearer ${accessToken}`)
  }

  if (CREDENTIAL_PATHS.has(path)) {
    config.withCredentials = true
    config.headers.set('Content-Type', 'application/json')
  }

  if (CSRF_PATHS.has(path)) {
    config.headers.set('X-Requested-With', 'XMLHttpRequest')
  }

  return config
})

function shouldRefresh(
  config: RetryableConfig | undefined,
  error: ApiError
): config is RetryableConfig {
  return (
    config !== undefined &&
    config._authRetry !== true &&
    error.status === 401 &&
    REFRESHABLE_CODES.has(error.code) &&
    !NO_REFRESH_PATHS.has(pathnameOf(config.url))
  )
}

apiClient.interceptors.response.use(
  (response) => response,
  async (error: unknown) => {
    if (axios.isCancel(error)) throw error

    const config = axios.isAxiosError(error)
      ? (error.config as RetryableConfig | undefined)
      : undefined
    const apiError = toApiError(error)

    if (apiError?.code === 'account_disabled') {
      authSession.handleAccountDisabled(apiError)
      throw apiError
    }

    if (apiError && shouldRefresh(config, apiError)) {
      const currentToken = authStore.getState().accessToken
      const usedToken = config.headers.get('Authorization')

      // Another request in the same burst may have refreshed while this 401
      // was in flight. Retry with the token that is already in memory instead
      // of rotating the refresh token a second time.
      if (currentToken && usedToken !== `Bearer ${currentToken}`) {
        config._authRetry = true
        config.headers.set('Authorization', `Bearer ${currentToken}`)
        return await apiClient.request(config)
      }

      try {
        const accessToken = await authSession.refreshAccessToken()
        config._authRetry = true
        config.headers.set('Authorization', `Bearer ${accessToken}`)
        return await apiClient.request(config)
      } catch (refreshError) {
        throw toApiError(refreshError) ?? refreshError
      }
    }

    throw apiError ?? error
  }
)

export const customInstance = async <T>(
  config: AxiosRequestConfig,
  options?: AxiosRequestConfig
): Promise<T> => {
  const response = await apiClient.request<T>({
    ...config,
    ...options,
    baseURL: apiBaseUrl,
  })

  return response.data
}
