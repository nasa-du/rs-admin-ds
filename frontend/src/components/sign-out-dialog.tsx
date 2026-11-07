import { useNavigate, useLocation } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { ConfirmDialog } from '@/components/confirm-dialog'

interface SignOutDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function SignOutDialog({ open, onOpenChange }: SignOutDialogProps) {
  const { t } = useTranslation('common')
  const { t: tAuth } = useTranslation('auth')
  const navigate = useNavigate()
  const location = useLocation()

  /**
   * The session API is wired in #40. Until then, sign-out only returns to the
   * sign-in screen while preserving the current path for the redirect.
   */
  const handleSignOut = () => {
    navigate({
      to: '/sign-in',
      search: { redirect: location.href },
      replace: true,
    })
  }

  return (
    <ConfirmDialog
      open={open}
      onOpenChange={onOpenChange}
      title={tAuth('signOut.title')}
      desc={tAuth('signOut.description')}
      confirmText={tAuth('signOut.confirm')}
      cancelBtnText={t('actions.cancel')}
      destructive
      handleConfirm={handleSignOut}
      className='sm:max-w-sm'
    />
  )
}
