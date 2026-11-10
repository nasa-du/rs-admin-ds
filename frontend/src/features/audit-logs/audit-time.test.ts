import { describe, expect, it } from 'vitest'
import {
  AUDIT_MAX_RANGE_MS,
  isAuditRangePreset,
  isValidAuditRange,
  presetWindow,
  resolveAuditWindow,
} from './audit-time'

const NOW = new Date('2026-11-10T12:00:00.000Z')

describe('presetWindow', () => {
  it('resolves every preset to an absolute UTC window ending now', () => {
    expect(presetWindow('15m', NOW)).toEqual({
      from: '2026-11-10T11:45:00.000Z',
      to: '2026-11-10T12:00:00.000Z',
    })
    expect(presetWindow('1h', NOW).from).toBe('2026-11-10T11:00:00.000Z')
    expect(presetWindow('24h', NOW).from).toBe('2026-11-09T12:00:00.000Z')
    expect(presetWindow('7d', NOW).from).toBe('2026-11-03T12:00:00.000Z')
    expect(presetWindow('30d', NOW).from).toBe('2026-10-11T12:00:00.000Z')
  })

  it('recognizes only the catalog of presets', () => {
    expect(isAuditRangePreset('30d')).toBe(true)
    expect(isAuditRangePreset('90d')).toBe(false)
    expect(isAuditRangePreset(undefined)).toBe(false)
  })
})

describe('resolveAuditWindow', () => {
  it('defaults to the last 30 days when the URL has no window', () => {
    expect(resolveAuditWindow({}, NOW)).toEqual({
      from: '2026-10-11T12:00:00.000Z',
      to: '2026-11-10T12:00:00.000Z',
      range: '30d',
    })
  })

  it('materializes a preset when only the range is in the URL', () => {
    expect(resolveAuditWindow({ range: '24h' }, NOW)).toEqual({
      from: '2026-11-09T12:00:00.000Z',
      to: '2026-11-10T12:00:00.000Z',
      range: '24h',
    })
  })

  it('keeps an absolute window from the URL unchanged', () => {
    expect(
      resolveAuditWindow(
        {
          from: '2026-01-01T00:00:00Z',
          to: '2026-01-02T00:00:00Z',
          range: '7d',
        },
        NOW
      )
    ).toEqual({
      from: '2026-01-01T00:00:00Z',
      to: '2026-01-02T00:00:00Z',
      range: '7d',
    })
  })

  it('fills only the missing bound using the server defaults', () => {
    expect(resolveAuditWindow({ from: '2026-01-01T00:00:00Z' }, NOW)).toEqual({
      from: '2026-01-01T00:00:00Z',
      to: '2026-11-10T12:00:00.000Z',
      range: undefined,
    })

    expect(resolveAuditWindow({ to: '2026-01-31T00:00:00Z' }, NOW)).toEqual({
      from: '2026-01-01T00:00:00.000Z',
      to: '2026-01-31T00:00:00Z',
      range: undefined,
    })
  })
})

describe('isValidAuditRange', () => {
  it('accepts an ascending window within the 90-day limit', () => {
    expect(
      isValidAuditRange(
        '2026-01-01T00:00:00Z',
        new Date(
          new Date('2026-01-01T00:00:00Z').getTime() + AUDIT_MAX_RANGE_MS
        ).toISOString()
      )
    ).toBe(true)
  })

  it('rejects a non-ascending or oversized window', () => {
    expect(
      isValidAuditRange('2026-01-02T00:00:00Z', '2026-01-01T00:00:00Z')
    ).toBe(false)
    expect(
      isValidAuditRange('2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')
    ).toBe(false)
    expect(
      isValidAuditRange(
        '2026-01-01T00:00:00Z',
        new Date(
          new Date('2026-01-01T00:00:00Z').getTime() + AUDIT_MAX_RANGE_MS + 1
        ).toISOString()
      )
    ).toBe(false)
  })

  it('rejects unparsable timestamps', () => {
    expect(isValidAuditRange('not-a-date', '2026-01-01T00:00:00Z')).toBe(false)
  })
})
