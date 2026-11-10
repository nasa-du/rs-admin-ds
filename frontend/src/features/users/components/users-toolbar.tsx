import { useEffect } from 'react'
import type { Table } from '@tanstack/react-table'
import { ListFilter, RefreshCw, Search, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { RoleSummary, UserSummary } from '@/api/generated/model'
import { roleDisplayName } from '@/lib/role-labels'
import { useSyncedValue } from '@/hooks/use-synced-value'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { DataTableViewOptions } from '@/components/data-table/view-options'
import {
  clearFiltersPatch,
  USERS_DEFAULT_SORT,
  type UsersSearch,
} from '../users-search'

const UUID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

type UsersToolbarProps = {
  table: Table<UserSummary>
  search: UsersSearch
  roleOptions: RoleSummary[]
  canReadRoles: boolean
  rolesLoading: boolean
  rolesError: boolean
  onRetryRoles: () => void
  onListChange: (patch: Partial<UsersSearch>) => void
  onRefresh: () => void
  isFetching: boolean
}

/**
 * Filter row for the users table (spec §9.8). The search box is debounced by
 * 300ms; every other filter writes the URL immediately. All list controls
 * reset to page 1 and close the detail sheet through `onListChange`.
 */
export function UsersToolbar({
  table,
  search,
  roleOptions,
  canReadRoles,
  rolesLoading,
  rolesError,
  onRetryRoles,
  onListChange,
  onRefresh,
  isFetching,
}: UsersToolbarProps) {
  const { t } = useTranslation('users')
  const { t: tRoles } = useTranslation('roles')
  const [searchInput, setSearchInput] = useSyncedValue(search.q ?? '')

  useEffect(() => {
    const trimmed = searchInput.trim()
    if (trimmed === (search.q ?? '')) return

    const timer = window.setTimeout(() => {
      onListChange({ q: trimmed || undefined })
    }, 300)
    return () => window.clearTimeout(timer)
  }, [searchInput, search.q, onListChange])

  const selectedRole = search.role_id
    ? roleOptions.find((role) => role.id === search.role_id)
    : undefined

  const roleLabel = selectedRole
    ? roleDisplayName(tRoles, selectedRole)
    : (search.role_id ?? '')

  const chipLabel = (label: string, value: string) =>
    t('filters.chipLabel', { label, value })
  const chips: Array<{ key: string; label: string; remove: () => void }> = []

  if (search.q) {
    chips.push({
      key: 'q',
      label: chipLabel(t('filters.search'), search.q),
      remove: () => onListChange({ q: undefined }),
    })
  }
  if (search.status) {
    chips.push({
      key: 'status',
      label: chipLabel(
        t('filters.status'),
        search.status === 'active' ? t('status.active') : t('status.disabled')
      ),
      remove: () => onListChange({ status: undefined }),
    })
  }
  if (search.role_id) {
    chips.push({
      key: 'role',
      label: chipLabel(t('filters.role'), roleLabel),
      remove: () => onListChange({ role_id: undefined }),
    })
  }
  if (search.is_locked !== undefined) {
    chips.push({
      key: 'locked',
      label: chipLabel(
        t('filters.isLocked'),
        search.is_locked ? t('filters.yes') : t('filters.no')
      ),
      remove: () => onListChange({ is_locked: undefined }),
    })
  }
  if (search.must_change_password !== undefined) {
    chips.push({
      key: 'must-change-password',
      label: chipLabel(
        t('filters.mustChangePassword'),
        search.must_change_password ? t('filters.yes') : t('filters.no')
      ),
      remove: () => onListChange({ must_change_password: undefined }),
    })
  }

  const moreFilterCount = [
    search.is_locked !== undefined,
    search.must_change_password !== undefined,
  ].filter(Boolean).length

  return (
    <div className='flex flex-col gap-2'>
      <div className='flex flex-wrap items-center gap-2'>
        <div className='relative min-w-56 flex-1 sm:max-w-xs'>
          <Search className='pointer-events-none absolute inset-s-2 top-1/2 size-4 -translate-y-1/2 text-muted-foreground' />
          <Input
            type='search'
            value={searchInput}
            onChange={(event) => setSearchInput(event.target.value)}
            placeholder={t('filters.searchPlaceholder')}
            aria-label={t('filters.searchPlaceholder')}
            className='h-8 ps-8'
          />
        </div>

        <Select
          value={search.status ?? 'all'}
          onValueChange={(value) =>
            onListChange({
              status:
                value === 'all' ? undefined : (value as 'active' | 'disabled'),
            })
          }
        >
          <SelectTrigger
            size='sm'
            className='h-8 w-36'
            aria-label={t('filters.status')}
          >
            <SelectValue placeholder={t('filters.allStatuses')} />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value='all'>{t('filters.allStatuses')}</SelectItem>
            <SelectItem value='active'>{t('status.active')}</SelectItem>
            <SelectItem value='disabled'>{t('status.disabled')}</SelectItem>
          </SelectContent>
        </Select>

        {canReadRoles ? (
          <Select
            value={search.role_id ?? 'all'}
            onValueChange={(value) =>
              onListChange({
                role_id: value === 'all' ? undefined : value,
              })
            }
            disabled={rolesLoading}
          >
            <SelectTrigger
              size='sm'
              className='h-8 w-40'
              aria-label={t('filters.role')}
            >
              <SelectValue
                placeholder={
                  rolesError
                    ? t('create.rolesLoadError')
                    : t('filters.allRoles')
                }
              />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value='all'>{t('filters.allRoles')}</SelectItem>
              {search.role_id &&
                !roleOptions.some((role) => role.id === search.role_id) && (
                  <SelectItem value={search.role_id}>
                    {search.role_id}
                  </SelectItem>
                )}
              {roleOptions.map((role) => (
                <SelectItem key={role.id} value={role.id}>
                  {roleDisplayName(tRoles, role)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        ) : (
          <RoleIdFilter
            value={search.role_id}
            onApply={(value) => onListChange({ role_id: value })}
          />
        )}

        {rolesError && canReadRoles && (
          <Button
            type='button'
            variant='ghost'
            size='sm'
            className='h-8'
            onClick={onRetryRoles}
          >
            {t('create.rolesRetry')}
          </Button>
        )}

        <Select
          value={search.sort ?? USERS_DEFAULT_SORT}
          onValueChange={(value) =>
            onListChange({
              sort: value === USERS_DEFAULT_SORT ? undefined : value,
            })
          }
        >
          <SelectTrigger
            size='sm'
            className='h-8 w-44'
            aria-label={t('filters.sort')}
          >
            <SelectValue placeholder={t('filters.sort')} />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={USERS_DEFAULT_SORT}>
              {t('sort.createdDesc')}
            </SelectItem>
            <SelectItem value='created_at'>{t('sort.createdAsc')}</SelectItem>
            <SelectItem value='-updated_at'>{t('sort.updatedDesc')}</SelectItem>
            <SelectItem value='updated_at'>{t('sort.updatedAsc')}</SelectItem>
            <SelectItem value='-last_login_at'>
              {t('sort.lastLoginDesc')}
            </SelectItem>
            <SelectItem value='last_login_at'>
              {t('sort.lastLoginAsc')}
            </SelectItem>
            <SelectItem value='username'>{t('sort.usernameAsc')}</SelectItem>
            <SelectItem value='-username'>{t('sort.usernameDesc')}</SelectItem>
            <SelectItem value='display_name'>
              {t('sort.displayNameAsc')}
            </SelectItem>
            <SelectItem value='-display_name'>
              {t('sort.displayNameDesc')}
            </SelectItem>
            <SelectItem value='email'>{t('sort.emailAsc')}</SelectItem>
            <SelectItem value='-email'>{t('sort.emailDesc')}</SelectItem>
            <SelectItem value='status'>{t('sort.statusAsc')}</SelectItem>
            <SelectItem value='-status'>{t('sort.statusDesc')}</SelectItem>
          </SelectContent>
        </Select>

        <Popover>
          <PopoverTrigger asChild>
            <Button
              type='button'
              variant='outline'
              size='sm'
              className='h-8 border-dashed'
            >
              <ListFilter className='size-4' />
              {t('filters.more')}
              {moreFilterCount > 0 && (
                <Badge variant='secondary'>{moreFilterCount}</Badge>
              )}
            </Button>
          </PopoverTrigger>
          <PopoverContent align='start' className='grid w-64 gap-3'>
            <TriStateFilter
              label={t('filters.isLocked')}
              value={search.is_locked}
              onChange={(value) => onListChange({ is_locked: value })}
            />
            <TriStateFilter
              label={t('filters.mustChangePassword')}
              value={search.must_change_password}
              onChange={(value) =>
                onListChange({ must_change_password: value })
              }
            />
          </PopoverContent>
        </Popover>

        <div className='ms-auto flex items-center gap-2'>
          <Button
            type='button'
            variant='outline'
            size='sm'
            className='h-8'
            onClick={onRefresh}
            disabled={isFetching}
            aria-label={t('actions.refresh')}
          >
            <RefreshCw
              className={isFetching ? 'size-4 animate-spin' : 'size-4'}
            />
            <span className='hidden sm:inline'>{t('actions.refresh')}</span>
          </Button>
          <DataTableViewOptions table={table} />
        </div>
      </div>

      {chips.length > 0 && (
        <div className='flex flex-wrap items-center gap-1'>
          {chips.map((chip) => (
            <Badge key={chip.key} variant='secondary' className='gap-1 pe-1'>
              {chip.label}
              <button
                type='button'
                className='rounded-sm p-0.5 hover:bg-background/60'
                aria-label={t('filters.remove', { filter: chip.label })}
                onClick={chip.remove}
              >
                <X className='size-3' />
              </button>
            </Badge>
          ))}
          <Button
            type='button'
            variant='ghost'
            size='sm'
            className='h-6 px-2 text-xs'
            onClick={() => onListChange(clearFiltersPatch())}
          >
            {t('filters.clearAll')}
          </Button>
        </div>
      )}
    </div>
  )
}

function RoleIdFilter({
  value,
  onApply,
}: {
  value: string | undefined
  onApply: (value: string | undefined) => void
}) {
  const { t } = useTranslation('users')
  const [input, setInput] = useSyncedValue(value ?? '')

  function apply() {
    const trimmed = input.trim()
    if (trimmed === '') {
      onApply(undefined)
      return
    }
    if (UUID_PATTERN.test(trimmed)) onApply(trimmed)
  }

  return (
    <Input
      value={input}
      onChange={(event) => setInput(event.target.value)}
      onBlur={apply}
      onKeyDown={(event) => {
        if (event.key === 'Enter') {
          event.preventDefault()
          apply()
        }
      }}
      placeholder={t('filters.roleIdPlaceholder')}
      aria-label={t('filters.roleId')}
      className='h-8 w-56 font-mono text-xs'
    />
  )
}

function TriStateFilter({
  label,
  value,
  onChange,
}: {
  label: string
  value: boolean | undefined
  onChange: (value: boolean | undefined) => void
}) {
  const { t } = useTranslation('users')
  const selectValue = value === undefined ? 'any' : value ? 'yes' : 'no'

  return (
    <div className='grid gap-1.5'>
      <Label>{label}</Label>
      <Select
        value={selectValue}
        onValueChange={(next) =>
          onChange(next === 'any' ? undefined : next === 'yes')
        }
      >
        <SelectTrigger size='sm' aria-label={label}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value='any'>{t('filters.any')}</SelectItem>
          <SelectItem value='yes'>{t('filters.yes')}</SelectItem>
          <SelectItem value='no'>{t('filters.no')}</SelectItem>
        </SelectContent>
      </Select>
    </div>
  )
}
