import { z } from 'zod'
import type { TFunction } from 'i18next'
import type {
  CreateRoleRequest,
  RoleDetail,
  UpdateRoleRequest,
} from '@/api/generated/model'
import type { SelectedPermissions } from './permission-catalog'

/**
 * Metadata, validation and dirty-diff rules for role create/edit (spec §9.9):
 * only dirty fields enter the PATCH body, and an empty description is sent as
 * an explicit `null` so the server clears it.
 */

export const ROLE_NAME_MAX_LENGTH = 64
export const ROLE_DESCRIPTION_MAX_LENGTH = 500

/**
 * Localized client validation (ADR-0007 §6): schemas are built from `t` so
 * language changes rebuild the messages, and every message lives under the
 * `validation` namespace.
 */
export function makeRoleMetadataSchema(t: TFunction<'validation'>) {
  return z.object({
    name: z
      .string()
      .trim()
      .min(1, t('required'))
      .max(
        ROLE_NAME_MAX_LENGTH,
        t('max_length', { max: ROLE_NAME_MAX_LENGTH })
      ),
    description: z
      .string()
      .trim()
      .max(
        ROLE_DESCRIPTION_MAX_LENGTH,
        t('max_length', { max: ROLE_DESCRIPTION_MAX_LENGTH })
      ),
  })
}

export function makeRolePermissionsSchema(t: TFunction<'validation'>) {
  return z.object({
    permissionKeys: z.array(z.string()).min(1, t('selectAtLeastOneItem')),
  })
}

const ROLE_PERMISSION_ERROR_CODES = new Set([
  'empty_permissions',
  'invalid_permission',
  'permission_subset_violation',
])

/** Server codes that belong on the permission matrix, not the form header. */
export function isRolePermissionErrorCode(code: string | undefined): boolean {
  return code !== undefined && ROLE_PERMISSION_ERROR_CODES.has(code)
}

export type RoleMetadataValues = {
  name: string
  description: string
}

function normalizeDescription(value: string): string | null {
  const trimmed = value.trim()
  return trimmed === '' ? null : trimmed
}

export function buildRoleUpdate(
  role: Pick<RoleDetail, 'name' | 'description'>,
  values: RoleMetadataValues
): UpdateRoleRequest | null {
  const patch: UpdateRoleRequest = {}

  const name = values.name.trim()
  if (name !== role.name) patch.name = name

  const description = normalizeDescription(values.description)
  if (description !== (role.description ?? null)) {
    patch.description = description
  }

  return Object.keys(patch).length > 0 ? patch : null
}

export function buildCreateRoleRequest(
  values: RoleMetadataValues,
  permissionKeys: readonly string[]
): CreateRoleRequest {
  const request: CreateRoleRequest = {
    name: values.name.trim(),
    permission_keys: [...permissionKeys],
  }

  const description = normalizeDescription(values.description)
  if (description !== null) request.description = description

  return request
}

export function permissionsDirty(
  rolePermissionKeys: readonly string[],
  selected: SelectedPermissions
): boolean {
  if (rolePermissionKeys.length !== selected.size) return true
  return rolePermissionKeys.some((key) => !selected.has(key))
}
