import { Outlet } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

/** Centered card layout shared by sign-in and change-password. */
export function AuthLayout() {
  const { t } = useTranslation()

  return (
    <div className='container grid h-svh max-w-none items-center justify-center'>
      <div className='mx-auto flex w-full flex-col justify-center space-y-2 py-8 sm:p-8'>
        <div className='mb-4 flex items-center justify-center'>
          <h1 className='text-xl font-medium'>{t('appName')}</h1>
        </div>
        <Outlet />
      </div>
    </div>
  )
}
