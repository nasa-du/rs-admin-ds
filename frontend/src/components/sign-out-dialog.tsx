import { useState } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { ConfirmDialog } from '@/components/confirm-dialog'
import { signOutOfApp } from '@/features/auth/sign-out'

interface SignOutDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function SignOutDialog({ open, onOpenChange }: SignOutDialogProps) {
  const { t } = useTranslation('common')
  const { t: tAuth } = useTranslation('auth')
  const [isPending, setIsPending] = useState(false)
  const navigate = useNavigate()

  async function handleSignOut() {
    setIsPending(true)
    try {
      // Local state is cleared by `performSignOut` regardless of the server
      // result; a failure only adds the "server session may still be active"
      // notice.
      await signOutOfApp()
    } catch {
      // Never keep the user on a protected page because sign-out failed.
    } finally {
      setIsPending(false)
      onOpenChange(false)
      void navigate({ to: '/sign-in', replace: true })
    }
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
      isLoading={isPending}
      handleConfirm={() => void handleSignOut()}
      className='sm:max-w-sm'
    />
  )
}
