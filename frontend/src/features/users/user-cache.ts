import type { QueryClient } from '@tanstack/react-query'
import {
  getGetUserQueryKey,
  getListUsersQueryKey,
} from '@/api/generated/rs-admin'
import { toApiError } from '@/lib/api-error'

/**
 * Invalidate the list and the open detail after a user management mutation
 * (spec §9.8): the detail Sheet stays open and re-reads the server's answer.
 */
export function invalidateUserQueries(
  queryClient: QueryClient,
  userId: string
): Promise<void> {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: getListUsersQueryKey() }),
    queryClient.invalidateQueries({ queryKey: getGetUserQueryKey(userId) }),
  ]).then(() => undefined)
}

/**
 * A stale target may have moved out of the operator's scope; re-read the
 * server's answer instead of guessing at the UI (spec §9.8).
 */
export function refreshUserDetailIfOutOfScope(
  queryClient: QueryClient,
  userId: string,
  error: unknown
): void {
  if (toApiError(error)?.code !== 'out_of_scope') return
  void queryClient.invalidateQueries({ queryKey: getGetUserQueryKey(userId) })
}
