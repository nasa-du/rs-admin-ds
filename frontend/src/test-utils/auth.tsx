import type { ReactNode } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { UserSelfResponse } from '@/api/generated/model'
import { authKeys } from '@/features/auth/auth-session'

/** Minimal current-user fixture for auth and permission tests. */
export function makeUser(
  overrides: Partial<UserSelfResponse> = {}
): UserSelfResponse {
  return {
    display_name: 'Test User',
    email: 'test@example.com',
    id: 'user-1',
    is_super_admin: false,
    must_change_password: false,
    permissions: [],
    roles: [],
    status: 'active',
    username: 'test',
    ...overrides,
  }
}

export function createAuthQueryClient(user: UserSelfResponse | null) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { gcTime: Infinity, retry: false } },
  })
  if (user) queryClient.setQueryData(authKeys.me, user)
  return queryClient
}

/**
 * Render wrapper that provides a QueryClient seeded with `['auth','me']`.
 * A `null` user models a session without a cached current user.
 */
export function authWrapper(user: UserSelfResponse | null) {
  const queryClient = createAuthQueryClient(user)

  return function AuthWrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    )
  }
}
