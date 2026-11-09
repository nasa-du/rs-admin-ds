import type { TFunction } from 'i18next'
import type { RoleSummary } from '@/api/generated/model'

type RoleIdentity = Pick<RoleSummary, 'is_builtin' | 'name'>

/** Localized built-in role name; unknown codes fall back to the stable code. */
export function builtinRoleLabel(t: TFunction<'roles'>, code: string): string {
  switch (code) {
    case 'super_admin':
      return t('builtin.super_admin')
    case 'admin':
      return t('builtin.admin')
    case 'user':
      return t('builtin.user')
    default:
      return code
  }
}

/** Display name for any role: localized for built-ins, stored name otherwise. */
export function roleDisplayName(
  t: TFunction<'roles'>,
  role: RoleIdentity
): string {
  return role.is_builtin ? builtinRoleLabel(t, role.name) : role.name
}

/** The baseline `user` role is always held and cannot be removed. */
export function isBaselineRole(role: RoleIdentity): boolean {
  return role.is_builtin && role.name === 'user'
}

/** Admin-level built-ins that need an extra warning when granted. */
export function isPrivilegedRole(role: RoleIdentity): boolean {
  return (
    role.is_builtin && (role.name === 'admin' || role.name === 'super_admin')
  )
}
