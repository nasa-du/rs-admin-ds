import { useSearch } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { UserAuthForm } from './components/user-auth-form'

export function SignIn() {
  const { t } = useTranslation('auth')
  const { redirect } = useSearch({ from: '/(auth)/sign-in' })

  return (
    <Card className='max-w-sm gap-4'>
      <CardHeader>
        <CardTitle className='text-lg tracking-tight'>
          {t('signIn.title')}
        </CardTitle>
        <CardDescription>{t('signIn.description')}</CardDescription>
      </CardHeader>
      <CardContent>
        <UserAuthForm redirect={redirect} />
      </CardContent>
    </Card>
  )
}
