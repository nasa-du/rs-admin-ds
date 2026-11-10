import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { RoleSummary, UserSelfResponse } from '@/api/generated/model'
import {
  isBaselineRole,
  isPrivilegedRole,
  roleDisplayName,
} from '@/lib/role-labels'
import { cn } from '@/lib/utils'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'

const FALLBACK_BASELINE: RoleSummary = {
  id: 'baseline-user',
  name: 'user',
  description: null,
  is_builtin: true,
  permission_keys: [],
  assignable: false,
  manageable: false,
  created_at: '',
  updated_at: '',
}

type RoleMultiSelectProps = {
  roles: RoleSummary[]
  value: string[]
  onChange: (roleIds: string[]) => void
  currentUser: UserSelfResponse | null
  isLoading?: boolean
  isError?: boolean
  onRetry?: () => void
  error?: string | null
  disabled?: boolean
  /**
   * Replaces the built-in privileged-role banner. Pass `null` to suppress it
   * when the parent decides a different condition applies.
   */
  privilegedWarning?: string | null
}

/**
 * Role picker shared by the create flow (and reusable by the later role
 * assignment dialog, spec §9.8). The baseline `user` role is always shown as
 * checked and disabled; only extra roles enter the submitted value.
 */
export function RoleMultiSelect({
  roles,
  value,
  onChange,
  currentUser,
  isLoading = false,
  isError = false,
  onRetry,
  error,
  disabled = false,
  privilegedWarning,
}: RoleMultiSelectProps) {
  const { t } = useTranslation('users')
  const { t: tRoles } = useTranslation('roles')
  const [search, setSearch] = useState('')

  const baseline = useMemo(
    () => roles.find(isBaselineRole) ?? FALLBACK_BASELINE,
    [roles]
  )

  const filtered = useMemo(() => {
    const query = search.trim().toLowerCase()
    return roles
      .filter((role) => !isBaselineRole(role))
      .filter((role) => !query || role.name.toLowerCase().includes(query))
  }, [roles, search])

  const privilegedSelected = value.some((id) => {
    const role = roles.find((candidate) => candidate.id === id)
    return role ? isPrivilegedRole(role) : false
  })

  const warning =
    privilegedWarning !== undefined
      ? privilegedWarning
      : privilegedSelected
        ? t('create.rolePrivilegedWarning')
        : null

  function disabledReason(role: RoleSummary): string | null {
    if (role.assignable) return null
    if (isPrivilegedRole(role) && !currentUser?.is_super_admin) {
      return t('create.rolePrivilegedOnly')
    }
    return t('create.roleNotAssignable')
  }

  function toggle(role: RoleSummary, checked: boolean) {
    if (checked) {
      onChange([...value, role.id])
    } else {
      onChange(value.filter((id) => id !== role.id))
    }
  }

  return (
    <div className='grid gap-2'>
      <div className='flex items-center justify-between gap-2'>
        <p className='text-sm font-medium'>{t('create.roles')}</p>
        <span className='text-xs text-muted-foreground'>
          {t('create.roleSelected', { count: value.length })}
        </span>
      </div>
      <p className='text-xs text-muted-foreground'>{t('create.rolesHint')}</p>

      {error && <p className='text-sm text-destructive'>{error}</p>}

      {isLoading ? (
        <div className='grid gap-2 rounded-md border p-3'>
          <Skeleton className='h-4 w-2/3' />
          <Skeleton className='h-4 w-1/2' />
          <Skeleton className='h-4 w-3/5' />
        </div>
      ) : isError ? (
        <div className='flex items-center justify-between gap-2 rounded-md border p-3 text-sm'>
          <span className='text-muted-foreground'>
            {t('create.rolesLoadError')}
          </span>
          {onRetry && (
            <Button type='button' variant='outline' size='sm' onClick={onRetry}>
              {t('create.rolesRetry')}
            </Button>
          )}
        </div>
      ) : (
        <>
          <Input
            type='search'
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder={t('create.rolesSearch')}
            aria-label={t('create.rolesSearch')}
            disabled={disabled}
          />
          <ScrollArea className='h-56 rounded-md border'>
            <div className='grid gap-1 p-2'>
              <RoleOption
                role={baseline}
                label={roleDisplayName(tRoles, baseline)}
                checked
                disabled
                selectedCountLabel={t('create.roleBaseline')}
                permissionCount={t('create.rolePermissionCount', {
                  count: baseline.permission_keys.length,
                })}
              />
              {filtered.map((role) => {
                const reason = disabledReason(role)
                return (
                  <RoleOption
                    key={role.id}
                    role={role}
                    label={roleDisplayName(tRoles, role)}
                    checked={value.includes(role.id)}
                    disabled={disabled || Boolean(reason)}
                    disabledReason={reason ?? undefined}
                    permissionCount={t('create.rolePermissionCount', {
                      count: role.permission_keys.length,
                    })}
                    onCheckedChange={(checked) => toggle(role, checked)}
                  />
                )
              })}
              {filtered.length === 0 && (
                <p className='p-2 text-sm text-muted-foreground'>
                  {t('create.rolesEmpty')}
                </p>
              )}
            </div>
          </ScrollArea>
        </>
      )}

      {warning && (
        <div
          role='status'
          className='rounded-md border border-amber-500/50 bg-amber-500/10 p-2 text-xs'
        >
          {warning}
        </div>
      )}
    </div>
  )
}

type RoleOptionProps = {
  role: RoleSummary
  label: string
  checked: boolean
  disabled: boolean
  disabledReason?: string
  permissionCount: string
  selectedCountLabel?: string
  onCheckedChange?: (checked: boolean) => void
}

function RoleOption({
  role,
  label,
  checked,
  disabled,
  disabledReason,
  permissionCount,
  selectedCountLabel,
  onCheckedChange,
}: RoleOptionProps) {
  const id = `role-option-${role.id}`

  return (
    <div
      className={cn(
        'flex items-start gap-2 rounded-md p-2',
        disabled && 'opacity-60'
      )}
    >
      <Checkbox
        id={id}
        checked={checked}
        disabled={disabled}
        onCheckedChange={(next) => onCheckedChange?.(next === true)}
        className='mt-0.5'
      />
      <Label htmlFor={id} className='grid cursor-pointer gap-0.5 font-normal'>
        <span className='flex flex-wrap items-center gap-1.5 text-sm font-medium'>
          {label}
          {role.is_builtin && (
            <Badge variant='outline' className='font-mono text-[10px]'>
              {role.name}
            </Badge>
          )}
          {selectedCountLabel && (
            <Badge variant='secondary'>{selectedCountLabel}</Badge>
          )}
        </span>
        <span className='text-xs text-muted-foreground'>{permissionCount}</span>
        {disabledReason && (
          <span className='text-xs text-muted-foreground'>
            {disabledReason}
          </span>
        )}
      </Label>
    </div>
  )
}
