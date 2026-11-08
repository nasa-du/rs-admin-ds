import type { ReactNode } from 'react'
import { type Permission } from '@/lib/permissions'
import { usePermission } from './use-auth'

type CanProps = {
  permission: Permission
  children: ReactNode
}

/**
 * Renders `children` only when the current user holds `permission`
 * (ADR-0006 §6). Missing action permissions hide the control; target-level
 * restrictions use a disabled control with a tooltip instead.
 */
export function Can({ permission, children }: CanProps) {
  return usePermission(permission) ? children : null
}
