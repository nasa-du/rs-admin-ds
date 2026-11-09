import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { RoleRef } from '@/api/generated/model'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { roleDisplayName } from '../role-labels'

type UserRoleChipsProps = {
  roles: RoleRef[]
  max?: number
}

/**
 * Role chips for the list and detail views (spec §9.8): built-in roles show a
 * localized label plus their stable code, custom roles keep their name. Long
 * lists collapse to `+N` with an inline expand toggle.
 */
export function UserRoleChips({ roles, max = 2 }: UserRoleChipsProps) {
  const { t } = useTranslation('roles')
  const [expanded, setExpanded] = useState(false)

  if (roles.length === 0) {
    return <span className='text-muted-foreground'>—</span>
  }

  const visible = expanded ? roles : roles.slice(0, max)
  const hiddenCount = roles.length - visible.length

  return (
    <div className='flex flex-wrap items-center gap-1'>
      {visible.map((role) => (
        <span key={role.id} className='inline-flex items-center gap-1'>
          <Badge variant='secondary'>{roleDisplayName(t, role)}</Badge>
          {role.is_builtin && (
            <Badge variant='outline' className='font-mono'>
              {role.name}
            </Badge>
          )}
        </span>
      ))}
      {!expanded && hiddenCount > 0 && (
        <Button
          type='button'
          variant='ghost'
          size='sm'
          className='h-5 px-1 text-xs'
          onClick={(event) => {
            event.stopPropagation()
            setExpanded(true)
          }}
        >
          +{hiddenCount}
        </Button>
      )}
    </div>
  )
}
