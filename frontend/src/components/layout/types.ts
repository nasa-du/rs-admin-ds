import { type LinkProps } from '@tanstack/react-router'
import { type ParseKeys } from 'i18next'
import { type Permission } from '@/lib/permissions'

type NavTitleKey = ParseKeys<'nav'>

type BaseNavItem = {
  titleKey: NavTitleKey
  badge?: string
  icon?: React.ElementType
  permission?: Permission
}

type NavLink = BaseNavItem & {
  url: LinkProps['to'] | (string & {})
  items?: never
}

type NavCollapsible = BaseNavItem & {
  items: (BaseNavItem & { url: LinkProps['to'] | (string & {}) })[]
  url?: never
}

type NavItem = NavCollapsible | NavLink

type NavGroup = {
  titleKey: NavTitleKey
  items: NavItem[]
}

export type { NavGroup, NavItem, NavCollapsible, NavLink }
