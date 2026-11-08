/**
 * Backend permission catalog (ADR-0002). Kept in sync with the API contract;
 * the server remains authoritative for every decision.
 */
export type Permission =
  | 'users.read'
  | 'users.create'
  | 'users.update'
  | 'users.set_status'
  | 'users.reset_password'
  | 'users.unlock'
  | 'users.assign_role'
  | 'sessions.revoke'
  | 'roles.read'
  | 'roles.create'
  | 'roles.update'
  | 'roles.delete'
  | 'audit.read'

export type PermissionHolder = {
  is_super_admin: boolean
  permissions: readonly string[]
}

/**
 * Frontend permission gate (ADR-0006 §6). A super admin always passes; the
 * server remains the final authority for every decision.
 */
export function hasPermission(
  user: PermissionHolder | null | undefined,
  permission: Permission
): boolean {
  if (!user) return false
  return user.is_super_admin || user.permissions.includes(permission)
}
