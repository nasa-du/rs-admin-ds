import type { PermissionGroup, PermissionItem } from '@/api/generated/model'

/**
 * Pure permission-catalog helpers behind the roles permission matrix
 * (spec §9.9). The catalog itself is authoritative and already ordered by the
 * backend; these helpers only decide how a selected key set maps onto it.
 */

type LocalizedNames = {
  key: string
  display_name_zh: string
  display_name_en: string
}

type PermissionGroupNames = Pick<
  PermissionGroup,
  'key' | 'display_name_zh' | 'display_name_en'
>

type PermissionNames = Pick<
  PermissionItem,
  'key' | 'display_name_zh' | 'display_name_en'
>

export type SelectedPermissions = ReadonlySet<string>

function isChineseLocale(locale: string): boolean {
  return locale.toLowerCase().startsWith('zh')
}

function localizedName(names: LocalizedNames, locale: string): string {
  const preferred = isChineseLocale(locale)
    ? names.display_name_zh
    : names.display_name_en
  return preferred.trim() || names.key
}

export function permissionDisplayName(
  permission: PermissionNames,
  locale: string
): string {
  return localizedName(permission, locale)
}

export function permissionGroupDisplayName(
  group: PermissionGroupNames,
  locale: string
): string {
  return localizedName(group, locale)
}

export function grantablePermissionKeys(group: PermissionGroup): string[] {
  return group.permissions
    .filter((permission) => permission.grantable)
    .map((permission) => permission.key)
}

export function selectedGrantableCount(
  group: PermissionGroup,
  selected: SelectedPermissions
): number {
  return group.permissions.filter(
    (permission) => permission.grantable && selected.has(permission.key)
  ).length
}

export type GroupSelectionState = 'all' | 'some' | 'none'

export function groupSelectionState(
  group: PermissionGroup,
  selected: SelectedPermissions
): GroupSelectionState {
  const grantable = grantablePermissionKeys(group)
  if (grantable.length === 0) return 'none'

  const selectedCount = selectedGrantableCount(group, selected)
  if (selectedCount === 0) return 'none'
  return selectedCount === grantable.length ? 'all' : 'some'
}

/**
 * Toggle a resource group's grantable permissions. Selections outside the
 * group and non-grantable selections are preserved; the group checkbox only
 * ever adds or removes grantable keys.
 */
export function togglePermissionGroup(
  group: PermissionGroup,
  selected: SelectedPermissions
): Set<string> {
  const next = new Set(selected)
  const grantable = grantablePermissionKeys(group)
  const allSelected =
    grantable.length > 0 && grantable.every((key) => selected.has(key))

  for (const key of grantable) {
    if (allSelected) {
      next.delete(key)
    } else {
      next.add(key)
    }
  }

  return next
}

/** Selected keys in catalog order, which is the order the API receives. */
export function orderedPermissionKeys(
  groups: readonly PermissionGroup[],
  selected: SelectedPermissions
): string[] {
  return groups.flatMap((group) =>
    group.permissions
      .map((permission) => permission.key)
      .filter((key) => selected.has(key))
  )
}

/** Every catalog key; used for the read-only super admin matrix. */
export function allPermissionKeys(
  groups: readonly PermissionGroup[]
): string[] {
  return groups.flatMap((group) =>
    group.permissions.map((permission) => permission.key)
  )
}
