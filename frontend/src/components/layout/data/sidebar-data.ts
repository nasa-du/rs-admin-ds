import { useMemo } from 'react'
import {
  Bell,
  LayoutDashboard,
  Monitor,
  Palette,
  ScrollText,
  Settings,
  ShieldCheck,
  UserCog,
  Users,
  Wrench,
} from 'lucide-react'
import { hasPermission, type Permission } from '@/lib/permissions'
import { useAuth } from '@/features/auth/use-auth'
import { type NavGroup, type NavItem } from '../types'

/**
 * Sidebar navigation (spec §9.2). Items without business permissions are
 * visible to every signed-in user; permissioned items are hidden when the
 * current user lacks the permission.
 */
const sidebarNavGroups: NavGroup[] = [
  {
    titleKey: 'groups.dashboard',
    items: [
      {
        titleKey: 'dashboard',
        url: '/',
        icon: LayoutDashboard,
      },
    ],
  },
  {
    titleKey: 'groups.management',
    items: [
      {
        titleKey: 'users',
        url: '/users',
        icon: Users,
        permission: 'users.read',
      },
      {
        titleKey: 'roles',
        url: '/roles',
        icon: ShieldCheck,
        permission: 'roles.read',
      },
      {
        titleKey: 'auditLogs',
        url: '/audit-logs',
        icon: ScrollText,
        permission: 'audit.read',
      },
    ],
  },
  {
    titleKey: 'groups.settings',
    items: [
      {
        titleKey: 'settings',
        icon: Settings,
        items: [
          {
            titleKey: 'profile',
            url: '/settings',
            icon: UserCog,
          },
          {
            titleKey: 'account',
            url: '/settings/account',
            icon: Wrench,
          },
          {
            titleKey: 'appearance',
            url: '/settings/appearance',
            icon: Palette,
          },
          {
            titleKey: 'notifications',
            url: '/settings/notifications',
            icon: Bell,
          },
          {
            titleKey: 'display',
            url: '/settings/display',
            icon: Monitor,
          },
        ],
      },
    ],
  },
]

export function useSidebarData(): NavGroup[] {
  return sidebarNavGroups
}

function isVisible(
  item: { permission?: Permission },
  can: (permission: Permission) => boolean
): boolean {
  return !item.permission || can(item.permission)
}

function filterItems(
  items: readonly NavItem[],
  can: (permission: Permission) => boolean
): NavItem[] {
  const visible: NavItem[] = []

  for (const item of items) {
    if (!isVisible(item, can)) continue

    if (item.items) {
      const subItems = item.items.filter((subItem) => isVisible(subItem, can))
      if (subItems.length === 0) continue
      visible.push({ ...item, items: subItems })
      continue
    }

    visible.push(item)
  }

  return visible
}

/**
 * Permission filtering for the sidebar and command menu. Groups and
 * collapsibles left with no visible children are removed entirely.
 */
export function filterSidebarData(
  groups: readonly NavGroup[],
  can: (permission: Permission) => boolean
): NavGroup[] {
  return groups
    .map((group) => ({
      ...group,
      items: filterItems(group.items, can),
    }))
    .filter((group) => group.items.length > 0)
}

/** Sidebar data filtered by the current user's `/me` permissions. */
export function useVisibleSidebarData(): NavGroup[] {
  const user = useAuth()

  return useMemo(
    () =>
      filterSidebarData(sidebarNavGroups, (permission) =>
        hasPermission(user, permission)
      ),
    [user]
  )
}
