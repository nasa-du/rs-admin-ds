import { describe, expect, it } from 'vitest'
import {
  buildCreateRoleRequest,
  buildRoleUpdate,
  permissionsDirty,
} from './role-form'

describe('buildRoleUpdate', () => {
  const role = {
    name: 'support',
    description: 'Support staff',
  }

  it('returns null when nothing changed', () => {
    expect(
      buildRoleUpdate(role, {
        name: ' support ',
        description: ' Support staff ',
      })
    ).toBeNull()
  })

  it('submits only the dirty metadata field', () => {
    expect(
      buildRoleUpdate(role, { name: 'support-2', description: 'Support staff' })
    ).toEqual({ name: 'support-2' })
    expect(
      buildRoleUpdate(role, { name: 'support', description: 'Help desk' })
    ).toEqual({ description: 'Help desk' })
  })

  it('clears the description with an explicit null', () => {
    expect(
      buildRoleUpdate(role, { name: 'support', description: '   ' })
    ).toEqual({ description: null })
  })

  it('keeps an already-empty description unchanged', () => {
    expect(
      buildRoleUpdate(
        { name: 'support', description: null },
        { name: 'support', description: '' }
      )
    ).toBeNull()
  })
})

describe('buildCreateRoleRequest', () => {
  it('trims the name, omits an empty description and keeps the keys', () => {
    expect(
      buildCreateRoleRequest({ name: '  support  ', description: '  ' }, [
        'users.read',
        'roles.read',
      ])
    ).toEqual({
      name: 'support',
      permission_keys: ['users.read', 'roles.read'],
    })
  })

  it('includes a non-empty description', () => {
    expect(
      buildCreateRoleRequest(
        { name: 'support', description: ' Support staff ' },
        ['users.read']
      )
    ).toEqual({
      name: 'support',
      description: 'Support staff',
      permission_keys: ['users.read'],
    })
  })
})

describe('permissionsDirty', () => {
  it('compares permission sets irrespective of order', () => {
    expect(
      permissionsDirty(
        ['users.read', 'roles.read'],
        new Set(['roles.read', 'users.read'])
      )
    ).toBe(false)
  })

  it('detects additions and removals', () => {
    expect(
      permissionsDirty(['users.read'], new Set(['users.read', 'roles.read']))
    ).toBe(true)
    expect(permissionsDirty(['users.read'], new Set())).toBe(true)
  })
})
