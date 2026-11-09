import { z } from 'zod'
import type { ListUsersParams } from '@/api/generated/model'

/**
 * `/users` URL state (spec §9.8). Parameter names mirror the backend query so
 * a deep link such as `/users?role_id=...` works without translation. Invalid
 * values are discarded and fall back to the API defaults instead of throwing.
 */

export const USERS_PAGE_SIZE_DEFAULT = 20
export const USERS_PAGE_SIZE_OPTIONS = [10, 20, 50, 100] as const

const USERS_SORT_FIELDS = [
  'username',
  'email',
  'display_name',
  'status',
  'created_at',
  'last_login_at',
  'updated_at',
] as const

export const USERS_DEFAULT_SORT = '-created_at'

const USERS_SORT_VALUES = [
  ...USERS_SORT_FIELDS,
  ...USERS_SORT_FIELDS.map((field) => `-${field}`),
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

const optionalUuid = z.preprocess(
  (value) => (typeof value === 'string' ? value.trim() : value),
  z.string().uuid()
)

export const usersSearchSchema = z.object({
  page: optionalPositiveInteger.optional().catch(undefined),
  page_size: optionalPageSize.optional().catch(undefined),
  q: optionalTrimmedString(200).optional().catch(undefined),
  status: z.enum(['active', 'disabled']).optional().catch(undefined),
  role_id: optionalUuid.optional().catch(undefined),
  is_locked: optionalBoolean.optional().catch(undefined),
  must_change_password: optionalBoolean.optional().catch(undefined),
  sort: z.enum(USERS_SORT_VALUES).optional().catch(undefined),
  user: optionalUuid.optional().catch(undefined),
})

export type UsersSearch = z.infer<typeof usersSearchSchema>

/**
 * URL search → `GET /users` query. Defaults are omitted so the URL and the
 * TanStack Query key stay canonical (`page=1`, `page_size=20`,
 * `sort=-created_at` are the server defaults).
 */
export function toListUsersParams(search: UsersSearch): ListUsersParams {
  return {
    q: search.q,
    status: search.status,
    role_id: search.role_id,
    is_locked: search.is_locked,
    must_change_password: search.must_change_password,
    page:
      search.page !== undefined && search.page > 1 ? search.page : undefined,
    page_size:
      search.page_size !== undefined &&
      search.page_size !== USERS_PAGE_SIZE_DEFAULT
        ? search.page_size
        : undefined,
    sort: search.sort !== USERS_DEFAULT_SORT ? search.sort : undefined,
  }
}

export function sortingFromSearch(
  search: UsersSearch
): Array<{ id: string; desc: boolean }> {
  const sort = search.sort ?? USERS_DEFAULT_SORT
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
  return value === USERS_DEFAULT_SORT ? undefined : value
}

export function hasActiveFilters(search: UsersSearch): boolean {
  return Boolean(
    search.q ||
    search.status ||
    search.role_id ||
    search.is_locked !== undefined ||
    search.must_change_password !== undefined
  )
}

/** Filter/sort changes reset to the first page and close the detail sheet. */
export function listFiltersPatch(
  patch: Partial<UsersSearch>
): Partial<UsersSearch> {
  return { page: undefined, user: undefined, ...patch }
}

export function paginationPatch(
  next: { pageIndex: number; pageSize: number },
  currentPageSize: number
): Partial<UsersSearch> {
  const pageSizeChanged = next.pageSize !== currentPageSize
  const pageNumber = next.pageIndex + 1

  return {
    page: !pageSizeChanged && pageNumber > 1 ? pageNumber : undefined,
    page_size:
      next.pageSize === USERS_PAGE_SIZE_DEFAULT ? undefined : next.pageSize,
    user: undefined,
  }
}

export function sortingPatch(
  sorting: ReadonlyArray<{ id: string; desc: boolean }>
): Partial<UsersSearch> {
  return {
    sort: sortFromSorting(sorting),
    page: undefined,
    user: undefined,
  }
}

export function clearFiltersPatch(): Partial<UsersSearch> {
  return {
    q: undefined,
    status: undefined,
    role_id: undefined,
    is_locked: undefined,
    must_change_password: undefined,
    page: undefined,
    user: undefined,
  }
}
