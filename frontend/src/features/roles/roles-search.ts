import { z } from 'zod'
import type { ListRolesParams } from '@/api/generated/model'

/**
 * `/roles` URL state (spec §9.9). Parameter names mirror the backend query so
 * deep links and refreshes reproduce the same list. Invalid values are
 * discarded and fall back to the API defaults instead of throwing.
 */

export const ROLES_PAGE_SIZE_DEFAULT = 20
export const ROLES_PAGE_SIZE_OPTIONS = [10, 20, 50, 100] as const

const ROLES_SORT_FIELDS = ['name', 'created_at', 'updated_at'] as const

export const ROLES_DEFAULT_SORT = 'name'

const ROLES_SORT_VALUES = [
  ...ROLES_SORT_FIELDS,
  ...ROLES_SORT_FIELDS.map((field) => `-${field}`),
] as const

function parseNumberLike(value: unknown): unknown {
  if (typeof value !== 'string') return value
  const trimmed = value.trim()
  if (trimmed === '') return undefined
  return Number(trimmed)
}

function parseBooleanLike(value: unknown): unknown {
  if (value === 'true' || value === true) return true
  if (value === 'false' || value === false) return false
  return value
}

const optionalPositiveInteger = z.preprocess(
  parseNumberLike,
  z.number().int().min(1)
)

const optionalPageSize = z.preprocess(
  parseNumberLike,
  z.number().int().min(1).max(200)
)

const optionalBoolean = z.preprocess(parseBooleanLike, z.boolean())

const optionalTrimmedString = (max: number) =>
  z.preprocess(
    (value) => (typeof value === 'string' ? value.trim() : value),
    z.string().min(1).max(max)
  )

export const rolesSearchSchema = z.object({
  page: optionalPositiveInteger.optional().catch(undefined),
  page_size: optionalPageSize.optional().catch(undefined),
  q: optionalTrimmedString(200).optional().catch(undefined),
  is_builtin: optionalBoolean.optional().catch(undefined),
  sort: z.enum(ROLES_SORT_VALUES).optional().catch(undefined),
})

export type RolesSearch = z.infer<typeof rolesSearchSchema>

/**
 * URL search → `GET /roles` query. Defaults are omitted so the URL and the
 * TanStack Query key stay canonical (`page=1`, `page_size=20`, `sort=name`
 * are the server defaults).
 */
export function toListRolesParams(search: RolesSearch): ListRolesParams {
  return {
    q: search.q,
    is_builtin: search.is_builtin,
    page:
      search.page !== undefined && search.page > 1 ? search.page : undefined,
    page_size:
      search.page_size !== undefined &&
      search.page_size !== ROLES_PAGE_SIZE_DEFAULT
        ? search.page_size
        : undefined,
    sort: search.sort !== ROLES_DEFAULT_SORT ? search.sort : undefined,
  }
}

export function sortingFromSearch(
  search: RolesSearch
): Array<{ id: string; desc: boolean }> {
  const sort = search.sort ?? ROLES_DEFAULT_SORT
  if (sort.startsWith('-')) {
    return [{ id: sort.slice(1), desc: true }]
  }
  return [{ id: sort, desc: false }]
}

export function sortFromSorting(
  sorting: ReadonlyArray<{ id: string; desc: boolean }>
): string | undefined {
  const first = sorting[0]
  if (!first) return undefined
  const value = `${first.desc ? '-' : ''}${first.id}`
  return value === ROLES_DEFAULT_SORT ? undefined : value
}

export function hasActiveFilters(search: RolesSearch): boolean {
  return Boolean(search.q || search.is_builtin !== undefined)
}

/** Filter changes reset to the first page. */
export function listFiltersPatch(
  patch: Partial<RolesSearch>
): Partial<RolesSearch> {
  return { page: undefined, ...patch }
}

export function paginationPatch(
  next: { pageIndex: number; pageSize: number },
  currentPageSize: number
): Partial<RolesSearch> {
  const pageSizeChanged = next.pageSize !== currentPageSize
  const pageNumber = next.pageIndex + 1

  return {
    page: !pageSizeChanged && pageNumber > 1 ? pageNumber : undefined,
    page_size:
      next.pageSize === ROLES_PAGE_SIZE_DEFAULT ? undefined : next.pageSize,
  }
}

export function sortingPatch(
  sorting: ReadonlyArray<{ id: string; desc: boolean }>
): Partial<RolesSearch> {
  return {
    sort: sortFromSorting(sorting),
    page: undefined,
  }
}

export function clearFiltersPatch(): Partial<RolesSearch> {
  return {
    q: undefined,
    is_builtin: undefined,
    page: undefined,
  }
}
