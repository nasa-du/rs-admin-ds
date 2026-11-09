import { useQuery } from '@tanstack/react-query'
import type { RoleSummary } from '@/api/generated/model'
import { listRoles } from '@/api/generated/rs-admin'

/**
 * Role options for the create dialog and the role filter (spec §9.8). The
 * list endpoint pages at 200 at a time; keep reading until every active role
 * is loaded so the selector never silently drops a role.
 */
const roleOptionsQueryKey = ['roles', 'options'] as const

export function useRoleOptions(enabled: boolean) {
  return useQuery({
    queryKey: roleOptionsQueryKey,
    queryFn: async ({ signal }): Promise<RoleSummary[]> => {
      const roles: RoleSummary[] = []
      let page = 1

      while (true) {
        const response = await listRoles(
          { page, page_size: 200, sort: 'name' },
          undefined,
          signal
        )
        roles.push(...response.items)
        if (response.items.length === 0 || roles.length >= response.total) {
          break
        }
        page += 1
      }

      return roles
    },
    enabled,
    staleTime: 5 * 60 * 1000,
  })
}
