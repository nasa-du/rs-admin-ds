import {
  type ErrorTranslator,
  translateApiError,
  translateApiErrorFields,
} from '@/i18n/errors'
import { toApiError } from '@/lib/api-error'
import { isRolePermissionErrorCode } from './role-form'

/**
 * Route a role API failure to the form control that should display it. The
 * two role forms share the same code and field-pointer rules (spec §9.9).
 */
export type RoleServerErrorMapping =
  | { kind: 'name'; message: string }
  | { kind: 'description'; message: string }
  | { kind: 'permissions'; message: string }
  | { kind: 'form'; message: string }

export function mapRoleServerError(
  t: ErrorTranslator,
  error: unknown
): RoleServerErrorMapping {
  const code = toApiError(error)?.code

  if (code === 'role_name_taken') {
    return { kind: 'name', message: translateApiError(t, error) }
  }
  if (isRolePermissionErrorCode(code)) {
    return { kind: 'permissions', message: translateApiError(t, error) }
  }

  for (const { field, message } of translateApiErrorFields(t, error)) {
    if (field === 'name' || field === 'description') {
      return { kind: field, message }
    }
  }

  return { kind: 'form', message: translateApiError(t, error) }
}
