import { describe, expect, it } from 'vitest'
import type { PermissionGroup } from '@/api/generated/model'
import {
  allPermissionKeys,
  grantablePermissionKeys,
  groupSelectionState,
  orderedPermissionKeys,
  permissionDisplayName,
  permissionGroupDisplayName,
  selectedGrantableCount,
  togglePermissionGroup,
} from './permission-catalog'

const CATALOG: PermissionGroup[] = [
  {
    key: 'users',
    display_name_zh: '用户',
    display_name_en: 'Users',
    permissions: [
      {
        key: 'users.read',
        display_name_zh: '查看用户',
        display_name_en: 'View users',
        grantable: true,
      },
      {
        key: 'users.update',
        display_name_zh: '编辑用户',
        display_name_en: 'Edit users',
        grantable: false,
      },
      {
        key: 'users.create',
        display_name_zh: '创建用户',
        display_name_en: 'Create users',
        grantable: true,
      },
    ],
  },
  {
    key: 'roles',
    display_name_zh: '角色',
    display_name_en: 'Roles',
    permissions: [
      {
        key: 'roles.read',
        display_name_zh: '查看角色',
        display_name_en: 'View roles',
        grantable: true,
      },
    ],
  },
]

describe('localized names', () => {
  it('picks the backend display name for the active locale', () => {
    const permission = CATALOG[0].permissions[0]

    expect(permissionDisplayName(permission, 'zh-CN')).toBe('查看用户')
    expect(permissionDisplayName(permission, 'en-US')).toBe('View users')
    expect(permissionGroupDisplayName(CATALOG[0], 'zh-CN')).toBe('用户')
    expect(permissionGroupDisplayName(CATALOG[0], 'en-US')).toBe('Users')
  })

  it('falls back to the stable key when a display name is missing', () => {
    const permission = {
      key: 'audit.read',
      display_name_zh: '',
      display_name_en: '',
      grantable: true,
    }

    expect(permissionDisplayName(permission, 'zh-CN')).toBe('audit.read')
    expect(permissionDisplayName(permission, 'en-US')).toBe('audit.read')
    expect(
      permissionGroupDisplayName(
        { key: 'audit', display_name_zh: '', display_name_en: '' },
        'en-US'
      )
    ).toBe('audit')
  })
})

describe('grantable selection', () => {
  it('lists only grantable permissions in catalog order', () => {
    expect(grantablePermissionKeys(CATALOG[0])).toEqual([
      'users.read',
      'users.create',
    ])
  })

  it('reports none, some and all for the grantable subset', () => {
    expect(groupSelectionState(CATALOG[0], new Set())).toBe('none')
    expect(groupSelectionState(CATALOG[0], new Set(['users.read']))).toBe(
      'some'
    )
    expect(
      groupSelectionState(CATALOG[0], new Set(['users.read', 'users.create']))
    ).toBe('all')
  })

  it('ignores selected permissions that are not grantable', () => {
    expect(groupSelectionState(CATALOG[0], new Set(['users.update']))).toBe(
      'none'
    )
    expect(
      selectedGrantableCount(
        CATALOG[0],
        new Set(['users.update', 'users.read'])
      )
    ).toBe(1)
  })

  it('selects every grantable permission and toggles them back off', () => {
    const selected = new Set<string>()
    const all = togglePermissionGroup(CATALOG[0], selected)

    expect([...all].sort()).toEqual(['users.create', 'users.read'])
    expect(togglePermissionGroup(CATALOG[0], all).size).toBe(0)
  })

  it('preserves selections outside the group and non-grantable selections', () => {
    const selected = new Set(['roles.read', 'users.update', 'users.read'])
    const next = togglePermissionGroup(CATALOG[0], selected)

    expect([...next].sort()).toEqual([
      'roles.read',
      'users.create',
      'users.read',
      'users.update',
    ])
  })

  it('returns permission keys in catalog order for submission', () => {
    const selected = new Set(['roles.read', 'users.create', 'users.read'])

    expect(orderedPermissionKeys(CATALOG, selected)).toEqual([
      'users.read',
      'users.create',
      'roles.read',
    ])
  })

  it('enumerates the full catalog for the built-in super admin role', () => {
    expect(allPermissionKeys(CATALOG)).toEqual([
      'users.read',
      'users.update',
      'users.create',
      'roles.read',
    ])
  })
})
