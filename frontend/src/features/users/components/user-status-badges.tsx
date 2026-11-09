import { useTranslation } from 'react-i18next'
import { Badge } from '@/components/ui/badge'

type UserStatusBadgesProps = {
  status: string
  lockedUntil?: string | null
  mustChangePassword: boolean
}

function isLocked(lockedUntil: string | null | undefined): boolean {
  if (!lockedUntil) return false
  const value = new Date(lockedUntil).getTime()
  return !Number.isNaN(value) && value > Date.now()
}

/** Status badge plus the independent lock and password-change markers. */
export function UserStatusBadges({
  status,
  lockedUntil,
  mustChangePassword,
}: UserStatusBadgesProps) {
  const { t } = useTranslation('users')

  return (
    <div className='flex flex-wrap items-center gap-1'>
      <Badge variant={status === 'active' ? 'default' : 'secondary'}>
        {status === 'disabled' ? t('status.disabled') : t('status.active')}
      </Badge>
      {isLocked(lockedUntil) && (
        <Badge variant='outline' className='text-destructive'>
          {t('status.locked')}
        </Badge>
      )}
      {mustChangePassword && (
        <Badge variant='outline'>{t('status.mustChangePassword')}</Badge>
      )}
    </div>
  )
}
