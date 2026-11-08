import { useQuery } from '@tanstack/react-query'
import type { UserSelfResponse } from '@/api/generated/model'
import { getMe } from '@/api/generated/rs-admin'
import { hasPermission, type Permission } from '@/lib/permissions'
import { authKeys } from './auth-session'
import { useAuthStore } from './auth-store'

/**
 * Current user and effective permissions, read from the `['auth','me']`
 * query cache populated by login or bootstrap (ADR-0006 §6). Refetches go
 * through the generated client so a 401 is refreshed and retried once, and
 * transient failures keep the cached user.
 */
export function useAuth(): UserSelfResponse | null {
  const isAuthenticated = useAuthStore(
    (state) => state.authStatus === 'authenticated'
  )

  const { data } = useQuery({
    queryKey: authKeys.me,
    queryFn: ({ signal }) => getMe(undefined, signal),
    enabled: isAuthenticated,
  })

  return data ?? null
}

/** Action-level permission gate; super admins always pass (spec §9.7). */
export function usePermission(permission: Permission): boolean {
  return hasPermission(useAuth(), permission)
}
