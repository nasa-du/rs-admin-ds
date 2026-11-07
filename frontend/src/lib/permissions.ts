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
