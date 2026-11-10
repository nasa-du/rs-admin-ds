import { useTranslation } from 'react-i18next'
import type { PermissionGroup } from '@/api/generated/model'
import { cn } from '@/lib/utils'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import { Label } from '@/components/ui/label'
import { DisabledTooltip } from '@/components/disabled-tooltip'
import {
  type SelectedPermissions,
  grantablePermissionKeys,
  groupSelectionState,
  permissionDisplayName,
  permissionGroupDisplayName,
  togglePermissionGroup,
} from '../permission-catalog'

type PermissionMatrixProps = {
  groups: PermissionGroup[]
  value: SelectedPermissions
  onChange: (next: Set<string>) => void
  readOnly?: boolean
}

/**
 * Full permission catalog grouped by resource (spec §9.9). Every permission
 * in the catalog is visible; non-grantable entries stay visible but disabled,
 * and each group has a tri-state toggle over its grantable subset only.
 */
export function PermissionMatrix({
  groups,
  value,
  onChange,
  readOnly = false,
}: PermissionMatrixProps) {
  const { t, i18n } = useTranslation('roles')
  const locale = i18n.language

  return (
    <div className='grid gap-4'>
      {groups.map((group) => {
        const groupName = permissionGroupDisplayName(group, locale)
        const state = groupSelectionState(group, value)
        const selectedCount = group.permissions.filter((permission) =>
          value.has(permission.key)
        ).length
        const hasGrantable = grantablePermissionKeys(group).length > 0

        return (
          <Card key={group.key} className='gap-4 py-4'>
            <CardHeader className='px-4'>
              <div className='flex flex-wrap items-center justify-between gap-2'>
                <div className='flex items-center gap-2'>
                  {!readOnly && (
                    <Checkbox
                      checked={
                        state === 'all'
                          ? true
                          : state === 'some'
                            ? 'indeterminate'
                            : false
                      }
                      disabled={!hasGrantable}
                      aria-label={t('matrix.selectGroup', {
                        group: groupName,
                      })}
                      onCheckedChange={() =>
                        onChange(togglePermissionGroup(group, value))
                      }
                    />
                  )}
                  <CardTitle className='text-base'>{groupName}</CardTitle>
                </div>
                <span className='text-xs text-muted-foreground'>
                  {t('matrix.selectedCount', { count: selectedCount })}
                </span>
              </div>
            </CardHeader>
            <CardContent className='grid gap-3 px-4 sm:grid-cols-2'>
              {group.permissions.map((permission) => {
                const name = permissionDisplayName(permission, locale)
                const disabled = readOnly || !permission.grantable
                const id = `permission-${permission.key}`

                return (
                  <div
                    key={permission.key}
                    className={cn(
                      'flex items-start gap-2',
                      !permission.grantable && 'opacity-60'
                    )}
                  >
                    <DisabledTooltip
                      reason={
                        permission.grantable
                          ? undefined
                          : t('matrix.notGrantable')
                      }
                      side='top'
                    >
                      <Checkbox
                        id={id}
                        checked={value.has(permission.key)}
                        disabled={disabled}
                        aria-label={name}
                        className='mt-0.5'
                        onCheckedChange={(next) => {
                          if (disabled) return
                          const updated = new Set(value)
                          if (next === true) {
                            updated.add(permission.key)
                          } else {
                            updated.delete(permission.key)
                          }
                          onChange(updated)
                        }}
                      />
                    </DisabledTooltip>
                    <Label
                      htmlFor={id}
                      className={cn(
                        'grid gap-0.5 font-normal',
                        disabled ? 'cursor-not-allowed' : 'cursor-pointer'
                      )}
                    >
                      <span className='text-sm'>{name}</span>
                      <span className='font-mono text-xs text-muted-foreground'>
                        {permission.key}
                      </span>
                      {!permission.grantable && (
                        <span className='text-xs text-muted-foreground'>
                          {t('matrix.notGrantable')}
                        </span>
                      )}
                    </Label>
                  </div>
                )
              })}
            </CardContent>
          </Card>
        )
      })}
    </div>
  )
}
