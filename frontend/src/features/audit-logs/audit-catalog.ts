import type auditResources from '@/i18n/locales/zh-CN/audit.json'
import type { TFunction } from 'i18next'

/**
 * Static audit catalogs (spec §9.10, §9.12).
 *
 * Backend action, target-type, severity and result values are stable codes.
 * The frontend maps the known catalog to localized labels and falls back to
 * the raw code for anything it does not know; translation keys are never
 * assembled from backend strings at runtime.
 */

export const AUDIT_ACTIONS = [
  'authn_login_success',
  'authn_login_fail',
  'authn_login_lock',
  'authn_login_rate_limited',
  'authn_logout',
  'authn_logout_all',
  'authn_refresh_reuse_detected',
  'authn_session_evicted',
  'authn_password_changed',
  'authn_password_change_fail',
  'authn_reauth_success',
  'authn_reauth_fail',
  'authz_step_up_denied',
  'authz_denied',
  'user_created',
  'user_updated',
  'user_status_changed',
  'user_password_reset',
  'user_unlocked',
  'user_roles_changed',
  'user_sessions_revoked',
  'role_created',
  'role_updated',
  'role_permissions_changed',
  'role_deleted',
  'system_bootstrap_super_admin',
  'system_recover_super_admin',
  'crypto_key_rotated',
  'system_audit_logs_purged',
  'audit_log_accessed',
] as const

export type KnownAuditAction = (typeof AUDIT_ACTIONS)[number]

type ResourcePaths<T> = T extends object
  ? {
      [K in keyof T & string]: T[K] extends object
        ? `${K}.${ResourcePaths<T[K]>}`
        : K
    }[keyof T & string]
  : never

type AuditKey = ResourcePaths<typeof auditResources>

export const AUDIT_TARGET_TYPES = [
  'user',
  'role',
  'session',
  'audit_log',
  'system',
] as const

export type KnownAuditTargetType = (typeof AUDIT_TARGET_TYPES)[number]

export const AUDIT_SEVERITIES = [
  'info',
  'warning',
  'error',
  'critical',
] as const

export type KnownAuditSeverity = (typeof AUDIT_SEVERITIES)[number]

export const AUDIT_RESULTS = ['success', 'failure', 'denied'] as const

export type KnownAuditResult = (typeof AUDIT_RESULTS)[number]

export type AuditActionGroupId =
  'authentication' | 'authorization' | 'user' | 'role' | 'system' | 'audit'

export const AUDIT_ACTION_GROUPS: ReadonlyArray<{
  id: AuditActionGroupId
  actions: readonly KnownAuditAction[]
}> = [
  {
    id: 'authentication',
    actions: [
      'authn_login_success',
      'authn_login_fail',
      'authn_login_lock',
      'authn_login_rate_limited',
      'authn_logout',
      'authn_logout_all',
      'authn_refresh_reuse_detected',
      'authn_session_evicted',
      'authn_password_changed',
      'authn_password_change_fail',
      'authn_reauth_success',
      'authn_reauth_fail',
    ],
  },
  {
    id: 'authorization',
    actions: ['authz_step_up_denied', 'authz_denied'],
  },
  {
    id: 'user',
    actions: [
      'user_created',
      'user_updated',
      'user_status_changed',
      'user_password_reset',
      'user_unlocked',
      'user_roles_changed',
      'user_sessions_revoked',
    ],
  },
  {
    id: 'role',
    actions: [
      'role_created',
      'role_updated',
      'role_permissions_changed',
      'role_deleted',
    ],
  },
  {
    id: 'system',
    actions: [
      'system_bootstrap_super_admin',
      'system_recover_super_admin',
      'crypto_key_rotated',
      'system_audit_logs_purged',
    ],
  },
  { id: 'audit', actions: ['audit_log_accessed'] },
]

const ACTION_KEYS = {
  authn_login_success: 'actions.authn_login_success',
  authn_login_fail: 'actions.authn_login_fail',
  authn_login_lock: 'actions.authn_login_lock',
  authn_login_rate_limited: 'actions.authn_login_rate_limited',
  authn_logout: 'actions.authn_logout',
  authn_logout_all: 'actions.authn_logout_all',
  authn_refresh_reuse_detected: 'actions.authn_refresh_reuse_detected',
  authn_session_evicted: 'actions.authn_session_evicted',
  authn_password_changed: 'actions.authn_password_changed',
  authn_password_change_fail: 'actions.authn_password_change_fail',
  authn_reauth_success: 'actions.authn_reauth_success',
  authn_reauth_fail: 'actions.authn_reauth_fail',
  authz_step_up_denied: 'actions.authz_step_up_denied',
  authz_denied: 'actions.authz_denied',
  user_created: 'actions.user_created',
  user_updated: 'actions.user_updated',
  user_status_changed: 'actions.user_status_changed',
  user_password_reset: 'actions.user_password_reset',
  user_unlocked: 'actions.user_unlocked',
  user_roles_changed: 'actions.user_roles_changed',
  user_sessions_revoked: 'actions.user_sessions_revoked',
  role_created: 'actions.role_created',
  role_updated: 'actions.role_updated',
  role_permissions_changed: 'actions.role_permissions_changed',
  role_deleted: 'actions.role_deleted',
  system_bootstrap_super_admin: 'actions.system_bootstrap_super_admin',
  system_recover_super_admin: 'actions.system_recover_super_admin',
  crypto_key_rotated: 'actions.crypto_key_rotated',
  system_audit_logs_purged: 'actions.system_audit_logs_purged',
  audit_log_accessed: 'actions.audit_log_accessed',
} as const satisfies Record<KnownAuditAction, AuditKey>

const TARGET_TYPE_KEYS = {
  user: 'targetTypes.user',
  role: 'targetTypes.role',
  session: 'targetTypes.session',
  audit_log: 'targetTypes.audit_log',
  system: 'targetTypes.system',
} as const satisfies Record<KnownAuditTargetType, AuditKey>

const SEVERITY_KEYS = {
  info: 'severity.info',
  warning: 'severity.warning',
  error: 'severity.error',
  critical: 'severity.critical',
} as const satisfies Record<KnownAuditSeverity, AuditKey>

const RESULT_KEYS = {
  success: 'results.success',
  failure: 'results.failure',
  denied: 'results.denied',
} as const satisfies Record<KnownAuditResult, AuditKey>

const ACTOR_KEYS = {
  anonymous: 'actors.anonymous',
  cli: 'actors.cli',
  system: 'actors.system',
} as const satisfies Record<string, AuditKey>

const ACTION_GROUP_KEYS = {
  authentication: 'actionGroups.authentication',
  authorization: 'actionGroups.authorization',
  user: 'actionGroups.user',
  role: 'actionGroups.role',
  system: 'actionGroups.system',
  audit: 'actionGroups.audit',
} as const satisfies Record<AuditActionGroupId, AuditKey>

const DETAIL_FIELD_KEYS = {
  permission_keys: 'detailFields.permissionKeys',
  role_ids: 'detailFields.roleIds',
  added_role_ids: 'detailFields.addedRoleIds',
  removed_role_ids: 'detailFields.removedRoleIds',
  added_permission_keys: 'detailFields.addedPermissionKeys',
  removed_permission_keys: 'detailFields.removedPermissionKeys',
  method: 'detailFields.method',
  reason: 'detailFields.reason',
  reason_code: 'detailFields.reasonCode',
  key_type: 'detailFields.keyType',
  old_kid: 'detailFields.oldKid',
  new_kid: 'detailFields.newKid',
  from_seq: 'detailFields.fromSeq',
  to_seq: 'detailFields.toSeq',
  count: 'detailFields.count',
  retention_days: 'detailFields.retentionDays',
  target_id: 'detailFields.targetId',
  actor_id: 'detailFields.actorId',
  request_id: 'detailFields.requestId',
  session_id: 'detailFields.sessionId',
  username: 'detailFields.username',
  email: 'detailFields.email',
  role_id: 'detailFields.roleId',
  user_id: 'detailFields.userId',
  view: 'detailFields.view',
  last_purged_hash: 'detailFields.lastPurgedHash',
  target_user_id: 'detailFields.targetUserId',
  changed_fields: 'detailFields.changedFields',
  from: 'detailFields.from',
  to: 'detailFields.to',
  sessions_revoked_count: 'detailFields.sessionsRevokedCount',
  failure_count: 'detailFields.failureCount',
  lock_seconds: 'detailFields.lockSeconds',
  source: 'detailFields.source',
  scope: 'detailFields.scope',
  retry_after_seconds: 'detailFields.retryAfterSeconds',
  rotation_counter: 'detailFields.rotationCounter',
  family_revoked: 'detailFields.familyRevoked',
  auth_method: 'detailFields.authMethod',
  auth_time: 'detailFields.authTime',
  must_change_password: 'detailFields.mustChangePassword',
  temporary_password_expires_at: 'detailFields.temporaryPasswordExpiresAt',
  cleared_failure_count: 'detailFields.clearedFailureCount',
  super_admin_involved: 'detailFields.superAdminInvolved',
  role_name: 'detailFields.roleName',
  identifier_hmac: 'detailFields.identifierHmac',
  operation: 'detailFields.operation',
  max_age_seconds: 'detailFields.maxAgeSeconds',
  created: 'detailFields.created',
  recovered: 'detailFields.recovered',
  evicted_session_id: 'detailFields.evictedSessionId',
  new_session_id: 'detailFields.newSessionId',
} as const satisfies Record<string, AuditKey>

type Lookup = Readonly<Record<string, AuditKey>>

function lookupLabel(t: TFunction<'audit'>, map: Lookup, code: string): string {
  const key = map[code]
  return key ? t(key) : code
}

export function isKnownAuditAction(value: string): value is KnownAuditAction {
  return (AUDIT_ACTIONS as readonly string[]).includes(value)
}

export function isKnownAuditTargetType(
  value: string
): value is KnownAuditTargetType {
  return (AUDIT_TARGET_TYPES as readonly string[]).includes(value)
}

/** Every known action in display order (grouped). */
export function groupedAuditActions(): KnownAuditAction[] {
  return AUDIT_ACTION_GROUPS.flatMap((group) => [...group.actions])
}

export function auditActionLabel(
  t: TFunction<'audit'>,
  action: string
): string {
  return lookupLabel(t, ACTION_KEYS, action)
}

export function auditTargetTypeLabel(
  t: TFunction<'audit'>,
  targetType: string
): string {
  return lookupLabel(t, TARGET_TYPE_KEYS, targetType)
}

export function auditSeverityLabel(
  t: TFunction<'audit'>,
  severity: string
): string {
  return lookupLabel(t, SEVERITY_KEYS, severity)
}

export function auditResultLabel(
  t: TFunction<'audit'>,
  result: string
): string {
  return lookupLabel(t, RESULT_KEYS, result)
}

export function auditActionGroupLabel(
  t: TFunction<'audit'>,
  group: AuditActionGroupId
): string {
  return t(ACTION_GROUP_KEYS[group])
}

/**
 * Actor labels are username snapshots. The three stable non-user markers are
 * localized; any other label is displayed exactly as recorded.
 */
export function auditActorLabel(
  t: TFunction<'audit'>,
  actor: { label: string }
): string {
  return lookupLabel(t, ACTOR_KEYS, actor.label)
}

export function auditDetailFieldLabel(
  t: TFunction<'audit'>,
  field: string
): string {
  return lookupLabel(t, DETAIL_FIELD_KEYS, field)
}
