import { useEffect } from 'react'
import type { Table as TanStackTable } from '@tanstack/react-table'
import { RefreshCw, Search, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { RoleSummary } from '@/api/generated/model'
import { useSyncedValue } from '@/hooks/use-synced-value'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { DataTableViewOptions } from '@/components/data-table/view-options'
import {
  ROLES_DEFAULT_SORT,
  clearFiltersPatch,
  type RolesSearch,
} from '../roles-search'

type RolesToolbarProps = {
  table: TanStackTable<RoleSummary>
  search: RolesSearch
  onListChange: (patch: Partial<RolesSearch>) => void
  onRefresh: () => void
  isFetching: boolean
}

/**
 * Filter row for the roles table (spec §9.9). The search box is debounced by
 * 300ms; every other filter writes the URL immediately. All list controls
 * reset to page 1 through `onListChange`.
 */
export function RolesToolbar({
  table,
  search,
  onListChange,
  onRefresh,
  isFetching,
}: RolesToolbarProps) {
  const { t } = useTranslation('roles')
  const [searchInput, setSearchInput] = useSyncedValue(search.q ?? '')

  useEffect(() => {
    const trimmed = searchInput.trim()
    if (trimmed === (search.q ?? '')) return

    const timer = window.setTimeout(() => {
      onListChange({ q: trimmed || undefined })
    }, 300)
    return () => window.clearTimeout(timer)
  }, [searchInput, search.q, onListChange])

  const isBuiltinLabel =
    search.is_builtin === undefined
      ? t('filters.allTypes')
      : search.is_builtin
        ? t('filters.builtin')
        : t('filters.custom')

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
          value={
            search.is_builtin === undefined
              ? 'all'
              : search.is_builtin
                ? 'builtin'
                : 'custom'
          }
          onValueChange={(value) =>
            onListChange({
              is_builtin: value === 'all' ? undefined : value === 'builtin',
            })
          }
        >
          <SelectTrigger
            size='sm'
            className='h-8 w-36'
            aria-label={t('filters.type')}
          >
            <SelectValue placeholder={t('filters.allTypes')} />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value='all'>{t('filters.allTypes')}</SelectItem>
            <SelectItem value='builtin'>{t('filters.builtin')}</SelectItem>
            <SelectItem value='custom'>{t('filters.custom')}</SelectItem>
          </SelectContent>
        </Select>

        <Select
          value={search.sort ?? ROLES_DEFAULT_SORT}
          onValueChange={(value) =>
            onListChange({
              sort: value === ROLES_DEFAULT_SORT ? undefined : value,
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
            <SelectItem value='name'>{t('sort.nameAsc')}</SelectItem>
            <SelectItem value='-name'>{t('sort.nameDesc')}</SelectItem>
            <SelectItem value='-updated_at'>{t('sort.updatedDesc')}</SelectItem>
            <SelectItem value='updated_at'>{t('sort.updatedAsc')}</SelectItem>
            <SelectItem value='-created_at'>{t('sort.createdDesc')}</SelectItem>
            <SelectItem value='created_at'>{t('sort.createdAsc')}</SelectItem>
          </SelectContent>
        </Select>

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

      {(search.q || search.is_builtin !== undefined) && (
        <div className='flex flex-wrap items-center gap-1'>
          {search.q && (
            <Badge variant='secondary' className='gap-1 pe-1'>
              {t('filters.chipLabel', {
                label: t('filters.search'),
                value: search.q,
              })}
              <button
                type='button'
                className='rounded-sm p-0.5 hover:bg-background/60'
                aria-label={t('filters.remove', {
                  filter: t('filters.search'),
                })}
                onClick={() => onListChange({ q: undefined })}
              >
                <X className='size-3' />
              </button>
            </Badge>
          )}
          {search.is_builtin !== undefined && (
            <Badge variant='secondary' className='gap-1 pe-1'>
              {t('filters.chipLabel', {
                label: t('filters.type'),
                value: isBuiltinLabel,
              })}
              <button
                type='button'
                className='rounded-sm p-0.5 hover:bg-background/60'
                aria-label={t('filters.remove', { filter: t('filters.type') })}
                onClick={() => onListChange({ is_builtin: undefined })}
              >
                <X className='size-3' />
              </button>
            </Badge>
          )}
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
