import { useEffect, useState } from 'react'
import { ChevronDown, Search, UserRound } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { useListUsers } from '@/api/generated/rs-admin'
import { isUuid } from '@/lib/uuid'
import { useSyncedValue } from '@/hooks/use-synced-value'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import { Skeleton } from '@/components/ui/skeleton'

type AuditActorFilterProps = {
  actorId: string | undefined
  canReadUsers: boolean
  onChange: (actorId: string | undefined) => void
}

/**
 * Actor filter (spec §9.10): a scoped user picker when the operator can read
 * users, plus an exact `actor_id` fallback for actors outside their scope.
 */
export function AuditActorFilter({
  actorId,
  canReadUsers,
  onChange,
}: AuditActorFilterProps) {
  const { t } = useTranslation('audit')
  const [open, setOpen] = useState(false)
  const [searchInput, setSearchInput] = useSyncedValue('')
  const [debounced, setDebounced] = useState('')
  const [idInput, setIdInput] = useSyncedValue(actorId ?? '')

  useEffect(() => {
    const timer = window.setTimeout(() => {
      setDebounced(searchInput.trim())
    }, 300)
    return () => window.clearTimeout(timer)
  }, [searchInput])

  const usersQuery = useListUsers(
    debounced ? { q: debounced, page_size: 20 } : undefined,
    { query: { enabled: open && canReadUsers && debounced.length > 0 } }
  )

  function selectActor(id: string | undefined) {
    onChange(id)
    setSearchInput('')
    setDebounced('')
    setOpen(false)
  }

  function applyExactId() {
    const trimmed = idInput.trim()
    if (trimmed === '') {
      selectActor(undefined)
      return
    }
    if (isUuid(trimmed)) selectActor(trimmed)
  }

  const triggerLabel = actorId ?? t('filters.allActors')

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          type='button'
          variant='outline'
          size='sm'
          className='h-8 max-w-64 justify-between gap-1'
          aria-label={t('filters.actor')}
        >
          <UserRound className='size-4 shrink-0' />
          <span className={actorId ? 'truncate font-mono text-xs' : 'truncate'}>
            {triggerLabel}
          </span>
          <ChevronDown className='size-4 shrink-0 opacity-50' />
        </Button>
      </PopoverTrigger>
      <PopoverContent align='start' className='grid w-80 gap-3'>
        {canReadUsers ? (
          <div className='grid gap-2'>
            <div className='relative'>
              <Search className='pointer-events-none absolute inset-s-2 top-1/2 size-4 -translate-y-1/2 text-muted-foreground' />
              <Input
                type='search'
                value={searchInput}
                onChange={(event) => setSearchInput(event.target.value)}
                placeholder={t('filters.actorSearchPlaceholder')}
                aria-label={t('filters.actorSearchPlaceholder')}
                className='h-8 ps-8'
              />
            </div>

            {debounced.length === 0 ? (
              <p className='text-xs text-muted-foreground'>
                {t('filters.actorSearchPlaceholder')}
              </p>
            ) : usersQuery.isPending ? (
              <div className='grid gap-1'>
                {Array.from({ length: 3 }, (_, index) => (
                  <Skeleton key={index} className='h-8 w-full' />
                ))}
              </div>
            ) : usersQuery.isError ? (
              <p className='text-xs text-destructive'>
                {t('filters.actorSearchError')}
              </p>
            ) : usersQuery.data?.items.length ? (
              <div className='grid max-h-56 gap-1 overflow-y-auto'>
                {usersQuery.data.items.map((user) => (
                  <button
                    key={user.id}
                    type='button'
                    className='grid gap-0.5 rounded-sm px-2 py-1 text-start hover:bg-accent'
                    onClick={() => selectActor(user.id)}
                  >
                    <span className='truncate text-sm font-medium'>
                      {user.display_name}
                    </span>
                    <span className='truncate text-xs text-muted-foreground'>
                      @{user.username} · {user.email}
                    </span>
                  </button>
                ))}
              </div>
            ) : (
              <p className='text-xs text-muted-foreground'>
                {t('filters.actorNoMatches')}
              </p>
            )}

            {actorId && (
              <Button
                type='button'
                variant='ghost'
                size='sm'
                className='w-fit'
                onClick={() => selectActor(undefined)}
              >
                {t('filters.allActors')}
              </Button>
            )}
          </div>
        ) : null}

        <div className='grid gap-1.5 border-t pt-3'>
          <Label htmlFor='audit-actor-id'>
            {t('filters.actorIdPlaceholder')}
          </Label>
          <div className='flex items-center gap-1'>
            <Input
              id='audit-actor-id'
              value={idInput}
              onChange={(event) => setIdInput(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  event.preventDefault()
                  applyExactId()
                }
              }}
              placeholder={t('filters.actorIdPlaceholder')}
              className='h-8 font-mono text-xs'
            />
            <Button type='button' size='sm' onClick={applyExactId}>
              {t('filters.actorApply')}
            </Button>
          </div>
        </div>
      </PopoverContent>
    </Popover>
  )
}
