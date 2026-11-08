import { QueryClient } from '@tanstack/react-query'
import { toApiError } from './api-error'
import { handleServerError } from './handle-server-error'

/**
 * Retry policy from ADR-0006 §3: never retry auth/permission/rate-limit
 * failures; retry network and 5xx read failures once; mutations never retry.
 */
function shouldRetry(failureCount: number, error: unknown): boolean {
  const status = toApiError(error)?.status

  if (status === 401 || status === 403 || status === 429) return false
  if (status === undefined) return failureCount < 1
  if (status >= 500) return failureCount < 1
  return false
}

function createAppQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: shouldRetry,
        refetchOnWindowFocus: import.meta.env.PROD,
        staleTime: 30 * 1000,
      },
      mutations: {
        retry: false,
        onError: (error) => {
          handleServerError(error)
        },
      },
    },
  })
}

/**
 * Shared cache. Login/logout/refresh-failure paths clear it to prevent data
 * from one user leaking into another session.
 */
export const queryClient = createAppQueryClient()
