import { describe, expect, it } from 'vitest'
import {
  AUDIT_DEFAULT_SORT,
  auditLogsSearchSchema,
  clearFiltersPatch,
  hasActiveFilters,
  isIpAddress,
  listFiltersPatch,
  paginationPatch,
  presetWindowPatch,
  sortingFromSearch,
  sortingPatch,
  sortFromSorting,
  toListAuditLogsParams,
} from './audit-logs-search'

const ACTOR_ID = '0190a1b2-0000-7000-8000-000000000001'
const TARGET_ID = '0190a1b2-0000-7000-8000-000000000002'
const DETAIL_ID = '0190a1b2-0000-7000-8000-000000000003'
const FROM = '2026-10-11T12:00:00.000Z'
const TO = '2026-11-10T12:00:00.000Z'

describe('auditLogsSearchSchema', () => {
  it('parses backend parameter names from a raw query string', () => {
    const parsed = auditLogsSearchSchema.parse({
      page: '2',
      page_size: '50',
      actor_id: ACTOR_ID,
      action: '  authn_login_success  ',
      target_type: ' user ',
      target_id: TARGET_ID,
      result: 'success',
      severity: 'info',
      request_id: ACTOR_ID,
      ip: ' 203.0.113.7 ',
      from: FROM,
      to: TO,
      sort: 'occurred_at',
      range: '30d',
      detail: DETAIL_ID,
    })

    expect(parsed).toEqual({
      page: 2,
      page_size: 50,
      actor_id: ACTOR_ID,
      action: 'authn_login_success',
      target_type: 'user',
      target_id: TARGET_ID,
      result: 'success',
      severity: 'info',
      request_id: ACTOR_ID,
      ip: '203.0.113.7',
      from: FROM,
      to: TO,
      sort: 'occurred_at',
      range: '30d',
      detail: DETAIL_ID,
    })
  })

  it('drops invalid structured values instead of throwing', () => {
    const parsed = auditLogsSearchSchema.parse({
      page: 0,
      page_size: 500,
      actor_id: 'not-a-uuid',
      result: 'maybe',
      severity: 'verbose',
      target_id: 'nope',
      request_id: 'nope',
      range: '90d',
      sort: 'actor',
      detail: 'nope',
    })

    expect(parsed.page).toBeUndefined()
    expect(parsed.page_size).toBeUndefined()
    expect(parsed.actor_id).toBeUndefined()
    expect(parsed.result).toBeUndefined()
    expect(parsed.severity).toBeUndefined()
    expect(parsed.target_id).toBeUndefined()
    expect(parsed.request_id).toBeUndefined()
    expect(parsed.range).toBeUndefined()
    expect(parsed.sort).toBeUndefined()
    expect(parsed.detail).toBeUndefined()
  })

  it('keeps unknown action and target-type codes for server-side tolerance', () => {
    const parsed = auditLogsSearchSchema.parse({
      action: 'future_action',
      target_type: 'future_target',
    })

    expect(parsed.action).toBe('future_action')
    expect(parsed.target_type).toBe('future_target')
  })
})

describe('toListAuditLogsParams', () => {
  it('passes the resolved absolute window and omits default paging', () => {
    expect(
      toListAuditLogsParams(
        { page: 1, page_size: 20, sort: AUDIT_DEFAULT_SORT },
        { from: FROM, to: TO, range: '30d' }
      )
    ).toEqual({
      actor_id: undefined,
      action: undefined,
      target_type: undefined,
      target_id: undefined,
      result: undefined,
      severity: undefined,
      request_id: undefined,
      ip: undefined,
      from: FROM,
      to: TO,
      page: undefined,
      page_size: undefined,
      sort: undefined,
    })
  })

  it('passes every filter through when set', () => {
    const params = toListAuditLogsParams(
      {
        page: 3,
        page_size: 50,
        actor_id: ACTOR_ID,
        action: 'authn_login_fail',
        target_type: 'user',
        target_id: TARGET_ID,
        result: 'failure',
        severity: 'warning',
        request_id: ACTOR_ID,
        ip: '203.0.113.7',
        sort: 'occurred_at',
      },
      { from: FROM, to: TO, range: undefined }
    )

    expect(params).toMatchObject({
      actor_id: ACTOR_ID,
      action: 'authn_login_fail',
      target_type: 'user',
      target_id: TARGET_ID,
      result: 'failure',
      severity: 'warning',
      request_id: ACTOR_ID,
      ip: '203.0.113.7',
      from: FROM,
      to: TO,
      page: 3,
      page_size: 50,
      sort: 'occurred_at',
    })
  })
})

describe('sorting helpers', () => {
  it('maps the default sort to descending occurred_at', () => {
    expect(sortingFromSearch({})).toEqual([{ id: 'occurred_at', desc: true }])
    expect(sortingFromSearch({ sort: 'occurred_at' })).toEqual([
      { id: 'occurred_at', desc: false },
    ])
  })

  it('omits the default sort from the URL', () => {
    expect(sortFromSorting([{ id: 'occurred_at', desc: true }])).toBeUndefined()
    expect(sortFromSorting([{ id: 'occurred_at', desc: false }])).toBe(
      'occurred_at'
    )
  })
})

describe('URL patches', () => {
  it('resets paging and closes the detail sheet on filter changes', () => {
    expect(listFiltersPatch({ action: 'role_created' })).toEqual({
      page: undefined,
      detail: undefined,
      action: 'role_created',
    })
  })

  it('resets or drops the page on pagination changes', () => {
    expect(paginationPatch({ pageIndex: 2, pageSize: 20 }, 20)).toMatchObject({
      page: 3,
      page_size: undefined,
      detail: undefined,
    })
    expect(paginationPatch({ pageIndex: 1, pageSize: 50 }, 20)).toMatchObject({
      page: undefined,
      page_size: 50,
      detail: undefined,
    })
  })

  it('sorts and resets to the first page', () => {
    expect(sortingPatch([{ id: 'occurred_at', desc: true }])).toEqual({
      sort: undefined,
      page: undefined,
      detail: undefined,
    })
  })

  it('clears filters without touching the absolute window', () => {
    expect(clearFiltersPatch()).toEqual({
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
    })
  })

  it('resolves a preset into an absolute window patch', () => {
    const now = new Date('2026-11-10T12:00:00.000Z')
    expect(presetWindowPatch('24h', now)).toEqual({
      range: '24h',
      from: '2026-11-09T12:00:00.000Z',
      to: '2026-11-10T12:00:00.000Z',
      page: undefined,
      detail: undefined,
    })
  })
})

describe('hasActiveFilters', () => {
  it('ignores the time window and paging', () => {
    expect(hasActiveFilters({ from: FROM, to: TO, page: 2 })).toBe(false)
    expect(hasActiveFilters({ action: 'role_created' })).toBe(true)
    expect(hasActiveFilters({})).toBe(false)
  })
})

describe('isIpAddress', () => {
  it('accepts IPv4 and IPv6 literals', () => {
    expect(isIpAddress('203.0.113.7')).toBe(true)
    expect(isIpAddress('::1')).toBe(true)
    expect(isIpAddress('2001:db8::1')).toBe(true)
  })

  it('rejects garbage, CIDR and hostnames', () => {
    expect(isIpAddress('999.0.0.1')).toBe(false)
    expect(isIpAddress('203.0.113.0/24')).toBe(false)
    expect(isIpAddress('example.com')).toBe(false)
    expect(isIpAddress('')).toBe(false)
  })
})
