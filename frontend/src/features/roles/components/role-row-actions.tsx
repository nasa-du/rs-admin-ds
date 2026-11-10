import { MoreHorizontal, Pencil, Shield, Trash2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { RoleSummary } from '@/api/generated/model'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { DisabledTooltip } from '@/components/disabled-tooltip'

type RoleRowActionsProps = {
  role: RoleSummary
  canUpdate: boolean
  canDelete: boolean
  onOpenRole: (id: string) => void
  onDeleteRole: (role: RoleSummary) => void
}

/**
 * Row actions for the roles list (spec §9.9): View is always available;
 * edit/delete are hidden without the action permission, disabled with a
 * reason when the target role is outside the operator's scope, and absent
 * for built-in roles.
 */
export function RoleRowActions({
  role,
  canUpdate,
  canDelete,
  onOpenRole,
  onDeleteRole,
}: RoleRowActionsProps) {
  const { t } = useTranslation('roles')
  const notManageableReason = role.manageable
    ? undefined
    : t('detail.notManageableBanner')

  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <Button
          type='button'
          variant='ghost'
          size='icon'
          className='size-8'
          aria-label={t('actions.more')}
          onClick={(event) => event.stopPropagation()}
        >
          <MoreHorizontal className='size-4' />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align='end'
        onClick={(event) => event.stopPropagation()}
      >
        <DropdownMenuItem onSelect={() => onOpenRole(role.id)}>
          <Shield className='size-4' />
          {t('actions.view')}
        </DropdownMenuItem>
        {canUpdate && !role.is_builtin && (
          <DisabledTooltip reason={notManageableReason}>
            <DropdownMenuItem
              disabled={!role.manageable}
              onSelect={() => onOpenRole(role.id)}
            >
              <Pencil className='size-4' />
              {t('actions.edit')}
            </DropdownMenuItem>
          </DisabledTooltip>
        )}
        {canDelete && !role.is_builtin && (
          <DisabledTooltip reason={notManageableReason}>
            <DropdownMenuItem
              disabled={!role.manageable}
              onSelect={() => onDeleteRole(role)}
            >
              <Trash2 className='size-4' />
              {t('actions.delete')}
            </DropdownMenuItem>
          </DisabledTooltip>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
