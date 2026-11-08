import { describe, expect, it } from 'vitest'
import { hasPermission } from './permissions'
import { isProtectedPath, requiredPermissionForPath } from './route-permissions'

describe('hasPermission', () => {
  it('grants everything to a super admin', () => {
    expect(
      hasPermission({ is_super_admin: true, permissions: [] }, 'users.read')
    ).toBe(true)
  })

  it('checks the effective permission list otherwise', () => {
    const user = {
      is_super_admin: false,
      permissions: ['users.read', 'roles.read'],
    }
    expect(hasPermission(user, 'users.read')).toBe(true)
    expect(hasPermission(user, 'users.create')).toBe(false)
  })

  it('denies when there is no user', () => {
    expect(hasPermission(null, 'users.read')).toBe(false)
  })
})

describe('requiredPermissionForPath', () => {
  it('maps each protected area to its route permission', () => {
    expect(requiredPermissionForPath('/users')).toBe('users.read')
    expect(requiredPermissionForPath('/users/42')).toBe('users.read')
    expect(requiredPermissionForPath('/roles')).toBe('roles.read')
    expect(requiredPermissionForPath('/roles/42')).toBe('roles.read')
    expect(requiredPermissionForPath('/audit-logs')).toBe('audit.read')
  })

  it('gives /roles/new its create permission', () => {
    expect(requiredPermissionForPath('/roles/new')).toBe('roles.create')
  })

  it('leaves dashboard, settings and unknown paths unguarded', () => {
    expect(requiredPermissionForPath('/')).toBeNull()
    expect(requiredPermissionForPath('/settings')).toBeNull()
    expect(requiredPermissionForPath('/settings/account')).toBeNull()
    expect(requiredPermissionForPath('/sign-in')).toBeNull()
    expect(requiredPermissionForPath('/users2')).toBeNull()
    expect(requiredPermissionForPath('/unknown')).toBeNull()
  })
})

describe('isProtectedPath', () => {
  it('treats application and change-password routes as protected', () => {
    expect(isProtectedPath('/')).toBe(true)
    expect(isProtectedPath('/users')).toBe(true)
    expect(isProtectedPath('/users/42')).toBe(true)
    expect(isProtectedPath('/roles/new')).toBe(true)
    expect(isProtectedPath('/audit-logs')).toBe(true)
    expect(isProtectedPath('/settings/account')).toBe(true)
    expect(isProtectedPath('/change-password')).toBe(true)
  })

  it('treats public pages and unknown paths as unprotected', () => {
    expect(isProtectedPath('/sign-in')).toBe(false)
    expect(isProtectedPath('/401')).toBe(false)
    expect(isProtectedPath('/403')).toBe(false)
    expect(isProtectedPath('/404')).toBe(false)
    expect(isProtectedPath('/500')).toBe(false)
    expect(isProtectedPath('/503')).toBe(false)
    expect(isProtectedPath('/unknown')).toBe(false)
  })
})
