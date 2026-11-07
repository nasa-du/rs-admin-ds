import { useNavigate } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'

export function MaintenanceError() {
  const { t } = useTranslation('errors')
  const navigate = useNavigate()

  return (
    <div className='h-svh'>
      <div className='m-auto flex h-full w-full flex-col items-center justify-center gap-2'>
        <h1 className='text-[7rem] leading-tight font-bold'>503</h1>
        <span className='font-medium'>{t('pages.maintenance.title')}</span>
        <p className='text-center text-muted-foreground'>
          {t('pages.maintenance.description')}
        </p>
        <div className='mt-6 flex gap-4'>
          <Button onClick={() => navigate({ to: '/' })}>
            {t('actions.backHome')}
          </Button>
        </div>
      </div>
    </div>
  )
}
