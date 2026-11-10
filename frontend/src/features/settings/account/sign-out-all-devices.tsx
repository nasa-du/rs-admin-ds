import { useState } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { ConfirmDialog } from '@/components/confirm-dialog'
import { signOutEverywhere } from '@/features/auth/sign-out'

/**
 * Revokes every session, including this one, then clears local state and
 * returns to sign-in (spec §9.11). A failed server request still signs the
 * user out locally; the server session may already be gone.
 */
export function SignOutAllDevices() {
  const { t } = useTranslation('settings')
  const navigate = useNavigate()
  const [open, setOpen] = useState(false)
  const [isPending, setIsPending] = useState(false)

  async function handleConfirm() {
    setIsPending(true)
    try {
      // `signOutEverywhere` clears local state even when the request fails.
      await signOutEverywhere()
    } finally {
      setIsPending(false)
      setOpen(false)
      void navigate({ to: '/sign-in', replace: true })
    }
  }

  return (
    <section className='space-y-4'>
      <div className='space-y-0.5'>
        <h3 className='text-lg font-medium'>{t('account.sessionsTitle')}</h3>
        <p className='text-sm text-muted-foreground'>
          {t('account.sessionsDescription')}
        </p>
      </div>

      <Button type='button' variant='destructive' onClick={() => setOpen(true)}>
        {t('account.signOutAll')}
      </Button>

      <ConfirmDialog
        open={open}
        onOpenChange={setOpen}
        title={t('account.signOutAllTitle')}
        desc={t('account.signOutAllDescription')}
        confirmText={t('account.signOutAll')}
        destructive
        isLoading={isPending}
        handleConfirm={() => void handleConfirm()}
        className='sm:max-w-sm'
      />
    </section>
  )
}
