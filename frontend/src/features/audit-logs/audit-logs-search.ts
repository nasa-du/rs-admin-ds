import { z } from 'zod'
import type { ListAuditLogsParams } from '@/api/generated/model'
import {
  AUDIT_RANGE_IDS,
  presetWindow,
  type AuditRangePresetId,
  type AuditWindow,
} from './audit-time'

/**
 * `/audit-logs` URL state (spec §9.10).
 *
 * Parameter names mirror the backend query so deep links and refreshes
 * reproduce the same list. `detail` keeps the detail Sheet in the URL; `range`
 * is UI-only and records which preset produced the absolute `from`/`to` pair.
 * Invalid structured values are discarded; open codes (action, target type)
 * pass through so the server stays authoritative.
 */

export const AUDIT_PAGE_SIZE_DEFAULT = 20
export const AUDIT_PAGE_SIZE_OPTIONS = [20, 50, 100] as const

export const AUDIT_DEFAULT_SORT = '-occurred_at'

const AUDIT_SORT_VALUES = ['occurred_at', '-occurred_at'] as const

type AuditSortValue = (typeof AUDIT_SORT_VALUES)[number]

function parseNumberLike(value: unknown): unknown {
  if (typeof value !== 'string') return value
  const trimmed = value.trim()
  if (trimmed === '') return undefined
  return Number(trimmed)
}

const optionalPositiveInteger = z.preprocess(
  parseNumberLike,
  z.number().int().min(1)
)

const optionalPageSize = z.preprocess(
  parseNumberLike,
  z.number().int().min(1).max(200)
)

const optionalTrimmedString = (max: number) =>
  z.preprocess(
    (value) => (typeof value === 'string' ? value.trim() : value),
    z.string().min(1).max(max)
  )

const optionalUuid = z.preprocess(
  (value) => (typeof value === 'string' ? value.trim() : value),
  z.string().uuid()
)

export const auditLogsSearchSchema = z.object({
  page: optionalPositiveInteger.optional().catch(undefined),
  page_size: optionalPageSize.optional().catch(undefined),
  actor_id: optionalUuid.optional().catch(undefined),
  action: optionalTrimmedString(100).optional().catch(undefined),
  target_type: optionalTrimmedString(100).optional().catch(undefined),
  target_id: optionalUuid.optional().catch(undefined),
  result: z.enum(['success', 'failure', 'denied']).optional().catch(undefined),
  severity: z
    .enum(['info', 'warning', 'error', 'critical'])
    .optional()
    .catch(undefined),
  request_id: optionalUuid.optional().catch(undefined),
  ip: optionalTrimmedString(64).optional().catch(undefined),
  from: optionalTrimmedString(64).optional().catch(undefined),
  to: optionalTrimmedString(64).optional().catch(undefined),
  range: z.enum(AUDIT_RANGE_IDS).optional().catch(undefined),
  sort: z.enum(AUDIT_SORT_VALUES).optional().catch(undefined),
  detail: optionalUuid.optional().catch(undefined),
})

export type AuditLogsSearch = z.infer<typeof auditLogsSearchSchema>

/**
 * URL search + resolved absolute window → `GET /audit-logs` query. Defaults
 * are omitted so the URL and the TanStack Query key stay canonical.
 */
export function toListAuditLogsParams(
  search: AuditLogsSearch,
  window: AuditWindow
): ListAuditLogsParams {
  return {
    actor_id: search.actor_id,
    action: search.action,
    target_type: search.target_type,
    target_id: search.target_id,
    result: search.result,
    severity: search.severity,
    request_id: search.request_id,
    ip: search.ip,
    from: window.from,
    to: window.to,
    page:
      search.page !== undefined && search.page > 1 ? search.page : undefined,
    page_size:
      search.page_size !== undefined &&
      search.page_size !== AUDIT_PAGE_SIZE_DEFAULT
        ? search.page_size
        : undefined,
    sort: search.sort !== AUDIT_DEFAULT_SORT ? search.sort : undefined,
  }
}

export function sortingFromSearch(
  search: AuditLogsSearch
): Array<{ id: string; desc: boolean }> {
  const sort = search.sort ?? AUDIT_DEFAULT_SORT
  if (sort.startsWith('-')) {
    return [{ id: sort.slice(1), desc: true }]
  }
  return [{ id: sort, desc: false }]
}

export function sortFromSorting(
  sorting: ReadonlyArray<{ id: string; desc: boolean }>
): AuditSortValue | undefined {
  const first = sorting[0]
  if (!first) return undefined
  const value = `${first.desc ? '-' : ''}${first.id}`
  return value === AUDIT_DEFAULT_SORT ? undefined : (value as AuditSortValue)
}

/** Filters only; the absolute window is not treated as a removable filter. */
export function hasActiveFilters(search: AuditLogsSearch): boolean {
  return Boolean(
    search.actor_id ||
    search.action ||
    search.target_type ||
    search.target_id ||
    search.result ||
    search.severity ||
    search.request_id ||
    search.ip
  )
}

/** Filter changes reset to the first page and close the detail Sheet. */
export function listFiltersPatch(
  patch: Partial<AuditLogsSearch>
): Partial<AuditLogsSearch> {
  return { page: undefined, detail: undefined, ...patch }
}

export function paginationPatch(
  next: { pageIndex: number; pageSize: number },
  currentPageSize: number
): Partial<AuditLogsSearch> {
  const pageSizeChanged = next.pageSize !== currentPageSize
  const pageNumber = next.pageIndex + 1

  return {
    page: !pageSizeChanged && pageNumber > 1 ? pageNumber : undefined,
    page_size:
      next.pageSize === AUDIT_PAGE_SIZE_DEFAULT ? undefined : next.pageSize,
    detail: undefined,
  }
}

export function sortingPatch(
  sorting: ReadonlyArray<{ id: string; desc: boolean }>
): Partial<AuditLogsSearch> {
  return {
    sort: sortFromSorting(sorting),
    page: undefined,
    detail: undefined,
  }
}

export function clearFiltersPatch(): Partial<AuditLogsSearch> {
  return {
    actor_id: undefined,
    action: undefined,
    target_type: undefined,
    target_id: undefined,
    result: undefined,
    severity: undefined,
    request_id: undefined,
    ip: undefined,
    page: undefined,
    detail: undefined,
  }
}

/** Resolve a preset into the absolute window written to the URL. */
export function presetWindowPatch(
  presetId: AuditRangePresetId,
  now: Date = new Date()
): Partial<AuditLogsSearch> {
  return {
    range: presetId,
    ...presetWindow(presetId, now),
    page: undefined,
    detail: undefined,
  }
}

/** Exact IPv4/IPv6 literal check for the filter input; CIDR is not supported. */
export function isIpAddress(value: string): boolean {
  const trimmed = value.trim()
  if (!trimmed) return false

  if (!trimmed.includes(':')) {
    const parts = trimmed.split('.')
    if (parts.length !== 4) return false
    return parts.every((part) => /^\d{1,3}$/.test(part) && Number(part) <= 255)
  }

  // At least two colons distinguishes IPv6 from a bare host name.
  if (trimmed.split(':').length < 3) return false
  return /^[0-9a-fA-F:.]+$/.test(trimmed)
}
