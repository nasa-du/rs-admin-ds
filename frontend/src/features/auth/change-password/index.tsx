import { useQueryClient } from '@tanstack/react-query'
import { useSearch } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import type { UserSelfResponse } from '@/api/generated/model'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { authKeys } from '@/features/auth/auth-session'
import { ChangePasswordForm } from './components/change-password-form'

export function ChangePassword() {
  const { t } = useTranslation('auth')
  const { redirect } = useSearch({ from: '/(auth)/change-password' })
  const queryClient = useQueryClient()
  const user = queryClient.getQueryData<UserSelfResponse>(authKeys.me)

  return (
    <Card className='max-w-sm gap-4'>
      <CardHeader>
        <CardTitle className='text-lg tracking-tight'>
          {t('changePassword.title')}
        </CardTitle>
        <CardDescription>{t('changePassword.description')}</CardDescription>
      </CardHeader>
      <CardContent>
        <ChangePasswordForm
          redirect={redirect}
          mustChangePassword={user?.must_change_password ?? false}
        />
      </CardContent>
    </Card>
  )
}
