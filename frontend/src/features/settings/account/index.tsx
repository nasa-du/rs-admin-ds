import { useTranslation } from 'react-i18next'
import { Separator } from '@/components/ui/separator'
import { ChangePasswordForm } from '@/features/auth/change-password/components/change-password-form'
import { ContentSection } from '../components/content-section'
import { AccountForm } from './account-form'
import { SignOutAllDevices } from './sign-out-all-devices'

export function SettingsAccount() {
  const { t } = useTranslation('settings')

  return (
    <ContentSection title={t('account.title')} desc={t('account.description')}>
      <div className='space-y-10'>
        <section className='space-y-4'>
          <div className='space-y-0.5'>
            <h3 className='text-lg font-medium'>
              {t('account.passwordTitle')}
            </h3>
            <p className='text-sm text-muted-foreground'>
              {t('account.passwordDescription')}
            </p>
          </div>
          <ChangePasswordForm redirect='/settings/account' />
        </section>

        <Separator />

        <SignOutAllDevices />

        <Separator />

        <AccountForm />
      </div>
    </ContentSection>
  )
}
