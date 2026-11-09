import { useState } from 'react'
import { type ErrorTranslator, translateApiError } from '@/i18n/errors'
import { Copy } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import type { UserDetail, UserSelfResponse } from '@/api/generated/model'
import { useGetUser } from '@/api/generated/rs-admin'
import { formatDate } from '@/lib/format'
import { Button } from '@/components/ui/button'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { Skeleton } from '@/components/ui/skeleton'
import { ApiErrorAlert } from '@/components/api-error-alert'
import { copyToClipboard } from '../copy-to-clipboard'
import { UserDetailActions } from './user-detail-actions'
import { UserRoleChips } from './user-role-chips'
import { UserStatusBadges } from './user-status-badges'

type UserDetailSheetProps = {
  userId: string | undefined
  onClose: () => void
  onEdit: (user: UserDetail) => void
  canEdit: boolean
  currentUser: UserSelfResponse | null
}

/**
 * Read-only user detail sheet bound to `?user=<id>` (spec §9.8). The detail
 * is always re-read through `GET /users/{id}`; closing the sheet clears the
 * URL parameter through `onClose`.
 */
export function UserDetailSheet({
  userId,
  onClose,
  onEdit,
  canEdit,
  currentUser,
}: UserDetailSheetProps) {
  const { t } = useTranslation('users')
  const { t: tCommon } = useTranslation('common')
  const { t: tErrors } = useTranslation('errors')
  const [idCopied, setIdCopied] = useState(false)

  const query = useGetUser(userId ?? '', {
    query: {
      enabled: Boolean(userId),
      staleTime: 0,
    },
  })

  if (!userId) return null

  const detail = query.data

  async function copyId(id: string) {
    if (await copyToClipboard(id)) {
      setIdCopied(true)
      toast.success(t('detail.idCopied'))
      return
    }
    toast.error(t('detail.copyFailed'))
  }

  return (
    <Sheet
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <SheetContent className='w-full overflow-y-auto sm:max-w-md'>
        <SheetHeader>
          <SheetTitle>{t('detail.title')}</SheetTitle>
          <SheetDescription>
            {detail
              ? t('detail.description', { username: detail.username })
              : t('detail.title')}
          </SheetDescription>
        </SheetHeader>

        {query.isPending && !detail ? (
          <div className='grid gap-3 px-4'>
            {Array.from({ length: 6 }, (_, index) => (
              <Skeleton key={index} className='h-5 w-full' />
            ))}
          </div>
        ) : query.isError ? (
          <div className='grid gap-3 px-4'>
            <ApiErrorAlert
              error={query.error}
              message={translateApiError(
                tErrors as ErrorTranslator,
                query.error
              )}
            />
            <Button
              type='button'
              variant='outline'
              onClick={() => void query.refetch()}
            >
              {tCommon('error.retry')}
            </Button>
          </div>
        ) : detail ? (
          <div className='grid gap-6 px-4'>
            <div className='grid gap-1'>
              <p className='text-lg font-semibold'>{detail.display_name}</p>
              <p className='text-sm text-muted-foreground'>
                @{detail.username}
              </p>
              <UserStatusBadges
                status={detail.status}
                lockedUntil={detail.locked_until}
                mustChangePassword={detail.must_change_password}
              />
            </div>

            <section className='grid gap-2'>
              <h3 className='text-sm font-semibold'>{t('detail.account')}</h3>
              <dl className='grid gap-1'>
                <DetailRow label={t('detail.id')}>
                  <span className='inline-flex items-center gap-2'>
                    <span className='font-mono text-xs break-all'>
                      {detail.id}
                    </span>
                    <Button
                      type='button'
                      variant='ghost'
                      size='icon'
                      className='size-6'
                      onClick={() => void copyId(detail.id)}
                      aria-label={t('detail.copyId')}
                    >
                      <Copy
                        className={
                          idCopied ? 'size-3.5 text-emerald-600' : 'size-3.5'
                        }
                      />
                    </Button>
                  </span>
                </DetailRow>
                <DetailRow label={t('detail.username')}>
                  {detail.username}
                </DetailRow>
                <DetailRow label={t('detail.email')}>{detail.email}</DetailRow>
                <DetailRow label={t('detail.phone')}>
                  {detail.phone ?? t('detail.notSet')}
                </DetailRow>
              </dl>
            </section>

            <section className='grid gap-2'>
              <h3 className='text-sm font-semibold'>{t('detail.status')}</h3>
              <dl className='grid gap-1'>
                <DetailRow label={t('detail.status')}>
                  <UserStatusBadges
                    status={detail.status}
                    lockedUntil={detail.locked_until}
                    mustChangePassword={detail.must_change_password}
                  />
                </DetailRow>
                <DetailRow label={t('detail.lockedUntil')}>
                  {detail.locked_until ? (
                    <DateValue value={detail.locked_until} />
                  ) : (
                    t('detail.notSet')
                  )}
                </DetailRow>
                <DetailRow label={t('detail.mustChangePassword')}>
                  {detail.must_change_password
                    ? t('status.mustChangePassword')
                    : t('detail.notSet')}
                </DetailRow>
                <DetailRow label={t('detail.temporaryPasswordExpiresAt')}>
                  {detail.temporary_password_expires_at ? (
                    <DateValue value={detail.temporary_password_expires_at} />
                  ) : (
                    t('detail.notSet')
                  )}
                </DetailRow>
              </dl>
            </section>

            <section className='grid gap-2'>
              <h3 className='text-sm font-semibold'>{t('detail.roles')}</h3>
              <UserRoleChips roles={detail.roles} max={6} />
            </section>

            <section className='grid gap-2'>
              <dl className='grid gap-1'>
                <DetailRow label={t('detail.lastLoginAt')}>
                  {detail.last_login_at ? (
                    <DateValue value={detail.last_login_at} />
                  ) : (
                    t('detail.neverSignedIn')
                  )}
                </DetailRow>
                <DetailRow label={t('detail.createdAt')}>
                  <DateValue value={detail.created_at} />
                </DetailRow>
                <DetailRow label={t('detail.updatedAt')}>
                  <DateValue value={detail.updated_at} />
                </DetailRow>
              </dl>
            </section>
          </div>
        ) : null}

        {detail && (
          <UserDetailActions
            user={detail}
            currentUser={currentUser}
            canEdit={canEdit}
            onEdit={onEdit}
          />
        )}
      </SheetContent>
    </Sheet>
  )
}

function DateValue({ value }: { value: string }) {
  return <span title={value}>{formatDate(value, { preset: 'dateTime' })}</span>
}

function DetailRow({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <div className='grid grid-cols-[7rem_1fr] gap-2 py-1 text-sm'>
      <dt className='text-muted-foreground'>{label}</dt>
      <dd className='min-w-0 break-words'>{children}</dd>
    </div>
  )
}
