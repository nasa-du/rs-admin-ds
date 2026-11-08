import { authWrapper, makeUser } from '@/test-utils/auth'
import { describe, expect, it } from 'vitest'
import { renderHook } from 'vitest-browser-react'
import { type NavGroup } from '../types'
import { filterSidebarData, useVisibleSidebarData } from './sidebar-data'

function managementTitles(groups: NavGroup[]): string[] {
  const management = groups.find(
    (group) => group.titleKey === 'groups.management'
  )
  return management?.items.map((item) => item.titleKey) ?? []
}

function settingsChildren(groups: NavGroup[]): string[] {
  const settings = groups.find((group) => group.titleKey === 'groups.settings')
  const collapsible = settings?.items[0]
  return collapsible?.items?.map((item) => item.titleKey) ?? []
}

describe('useVisibleSidebarData', () => {
  it('shows Dashboard without any business permission', async () => {
    const { result } = await renderHook(() => useVisibleSidebarData(), {
      wrapper: authWrapper(makeUser()),
    })

    const dashboard = result.current.find(
      (group) => group.titleKey === 'groups.dashboard'
    )
    expect(dashboard?.items.map((item) => item.titleKey)).toEqual(['dashboard'])
  })

  it('shows every management page to a super admin', async () => {
    const { result } = await renderHook(() => useVisibleSidebarData(), {
      wrapper: authWrapper(makeUser({ is_super_admin: true })),
    })

    expect(managementTitles(result.current)).toEqual([
      'users',
      'roles',
      'auditLogs',
    ])
  })

  it('keeps only the management pages covered by the permission list', async () => {
    const { result } = await renderHook(() => useVisibleSidebarData(), {
      wrapper: authWrapper(makeUser({ permissions: ['users.read'] })),
    })

    expect(managementTitles(result.current)).toEqual(['users'])
  })

  it('drops the management group when no page is permitted', async () => {
    const { result } = await renderHook(() => useVisibleSidebarData(), {
      wrapper: authWrapper(makeUser()),
    })

    expect(result.current.map((group) => group.titleKey)).toEqual([
      'groups.dashboard',
      'groups.settings',
    ])
  })

  it('keeps all five settings pages for any signed-in user', async () => {
    const { result } = await renderHook(() => useVisibleSidebarData(), {
      wrapper: authWrapper(makeUser()),
    })

    expect(settingsChildren(result.current)).toEqual([
      'profile',
      'account',
      'appearance',
      'notifications',
      'display',
    ])
  })
})

describe('filterSidebarData', () => {
  const groups: NavGroup[] = [
    {
      titleKey: 'groups.management',
      items: [
        {
          titleKey: 'users',
          url: '/users',
          permission: 'users.read',
        },
        {
          titleKey: 'roles',
          url: '/roles',
          permission: 'roles.read',
        },
      ],
    },
    {
      titleKey: 'groups.settings',
      items: [
        {
          titleKey: 'settings',
          items: [
            {
              titleKey: 'profile',
              url: '/settings',
            },
            {
              titleKey: 'account',
              url: '/settings/account',
              permission: 'sessions.revoke',
            },
          ],
        },
      ],
    },
  ]

  it('filters permissioned items and drops groups left empty', () => {
    const visible = filterSidebarData(
      groups,
      (permission) => permission === 'users.read'
    )

    expect(visible.map((group) => group.titleKey)).toEqual([
      'groups.management',
      'groups.settings',
    ])
    expect(visible[0].items.map((item) => item.titleKey)).toEqual(['users'])
    expect(visible[1].items[0].items?.map((item) => item.titleKey)).toEqual([
      'profile',
    ])
  })

  it('drops a collapsible whose children are all hidden', () => {
    const hidden = filterSidebarData(
      [
        {
          titleKey: 'groups.settings',
          items: [
            {
              titleKey: 'settings',
              items: [
                {
                  titleKey: 'account',
                  url: '/settings/account',
                  permission: 'sessions.revoke',
                },
              ],
            },
          ],
        },
      ],
      () => false
    )

    expect(hidden).toEqual([])
  })

  it('does not mutate the source data', () => {
    filterSidebarData(groups, () => false)

    expect(groups[0].items).toHaveLength(2)
    expect(groups[1].items[0].items).toHaveLength(2)
  })
})
