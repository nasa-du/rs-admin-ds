import {
  Bell,
  LayoutDashboard,
  Monitor,
  Palette,
  Settings,
  UserCog,
  Users,
  Wrench,
} from 'lucide-react'
import { type Permission } from '@/lib/permissions'
import { type NavGroup, type NavItem } from '../types'

const sidebarNavGroups: NavGroup[] = [
  {
    titleKey: 'groups.management',
    items: [
      {
        titleKey: 'dashboard',
        url: '/',
        icon: LayoutDashboard,
      },
      {
        titleKey: 'users',
        url: '/users',
        icon: Users,
        permission: 'users.read',
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

function isAllowed(item: NavItem, can: (permission: Permission) => boolean) {
  return !item.permission || can(item.permission)
}

/**
 * Filters the sidebar by a permission predicate. Until auth state lands, the
 * default predicate keeps all entries visible; #41 wires it to `/me`.
 */
export function useVisibleSidebarData(
  can: (permission: Permission) => boolean = () => true
): NavGroup[] {
  return useSidebarData()
    .map((group) => ({
      ...group,
      items: group.items
        .filter((item) => isAllowed(item, can))
        .map((item) =>
          item.items
            ? {
                ...item,
                items: item.items.filter((subItem) => isAllowed(subItem, can)),
              }
            : item
        ),
    }))
    .filter((group) => group.items.length > 0)
}
