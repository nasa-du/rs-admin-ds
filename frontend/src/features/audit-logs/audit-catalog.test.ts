import i18n from '@/i18n'
import type { TFunction } from 'i18next'
import { describe, expect, it } from 'vitest'
import {
  AUDIT_ACTION_GROUPS,
  AUDIT_ACTIONS,
  auditActionGroupLabel,
  auditActionLabel,
  auditActorLabel,
  auditDetailFieldLabel,
  auditResultLabel,
  auditSeverityLabel,
  auditTargetTypeLabel,
  groupedAuditActions,
} from './audit-catalog'

const en = i18n.getFixedT('en-US', 'audit') as TFunction<'audit'>
const zh = i18n.getFixedT('zh-CN', 'audit') as TFunction<'audit'>

describe('AUDIT_ACTIONS', () => {
  it('catalogues every known action exactly once across the groups', () => {
    const grouped = groupedAuditActions()
    expect(grouped).toEqual([...AUDIT_ACTIONS])
    expect(new Set(grouped).size).toBe(grouped.length)
    expect(grouped).toHaveLength(30)
  })
})

describe('localized labels', () => {
  it('maps known actions to localized labels', () => {
    expect(auditActionLabel(en, 'authn_login_success')).toBe('Signed in')
    expect(auditActionLabel(zh, 'authn_login_success')).toBe('登录成功')
    expect(auditActionLabel(en, 'audit_log_accessed')).toBe(
      'Audit details viewed'
    )
  })

  it('falls back to the raw code for unknown actions', () => {
    expect(auditActionLabel(en, 'future_action')).toBe('future_action')
    expect(auditActionLabel(zh, 'future_action')).toBe('future_action')
  })

  it('labels target types, severities and results with raw fallbacks', () => {
    expect(auditTargetTypeLabel(en, 'audit_log')).toBe('Audit log')
    expect(auditTargetTypeLabel(en, 'future_target')).toBe('future_target')
    expect(auditSeverityLabel(en, 'critical')).toBe('Critical')
    expect(auditSeverityLabel(en, 'future_severity')).toBe('future_severity')
    expect(auditResultLabel(zh, 'denied')).toBe('拒绝')
    expect(auditResultLabel(zh, 'future_result')).toBe('future_result')
  })

  it('labels stable actor markers and keeps user labels as recorded', () => {
    expect(auditActorLabel(en, { label: 'anonymous' })).toBe('Anonymous')
    expect(auditActorLabel(en, { label: 'cli' })).toBe('CLI')
    expect(auditActorLabel(zh, { label: 'system' })).toBe('系统')
    expect(auditActorLabel(en, { label: 'alice' })).toBe('alice')
  })

  it('interpolates non-plural counts in the detail view', () => {
    expect(en('detail.showMore', { count: 2 })).toBe('Show 2 more')
    expect(zh('detail.showMore', { count: 2 })).toBe('再显示 2 项')
  })

  it('labels action groups and detail fields', () => {
    expect(auditActionGroupLabel(en, 'authentication')).toBe('Authentication')
    expect(auditDetailFieldLabel(en, 'permission_keys')).toBe('Permissions')
    expect(auditDetailFieldLabel(zh, 'role_ids')).toBe('角色 ID')
    expect(auditDetailFieldLabel(en, 'future_field')).toBe('future_field')
  })
})

describe('AUDIT_ACTION_GROUPS', () => {
  it('groups actions by domain in display order', () => {
    expect(AUDIT_ACTION_GROUPS.map((group) => group.id)).toEqual([
      'authentication',
      'authorization',
      'user',
      'role',
      'system',
      'audit',
    ])
    expect(AUDIT_ACTION_GROUPS[0].actions).toContain('authn_login_success')
    expect(AUDIT_ACTION_GROUPS[5].actions).toEqual(['audit_log_accessed'])
  })
})
