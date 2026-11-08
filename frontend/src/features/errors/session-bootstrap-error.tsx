import { type ErrorComponentProps, useRouter } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { GeneralError } from './general-error'
import { SessionBootstrapUnavailableError } from './session-bootstrap-unavailable-error'

/**
 * Retryable bootstrap failure for protected routes: a network/5xx failure
 * must not be reported as "signed out" (ADR-0006 §1).
 */
export function SessionBootstrapError({ reset }: ErrorComponentProps) {
  const { t } = useTranslation('errors')
  const { t: tCommon } = useTranslation('common')
  const router = useRouter()

  return (
    <div className='h-svh'>
      <div className='m-auto flex h-full w-full flex-col items-center justify-center gap-2'>
        <span className='font-medium'>{t('pages.session.title')}</span>
        <p className='max-w-md text-center text-muted-foreground'>
          {t('pages.session.description')}
        </p>
        <Button
          className='mt-6'
          onClick={() => {
            reset()
            void router.invalidate()
          }}
        >
          {tCommon('error.retry')}
        </Button>
      </div>
    </div>
  )
}

/** Route-level error boundary: only bootstrap failures get the retry page. */
export function SessionBootstrapAwareError(props: ErrorComponentProps) {
  if (props.error instanceof SessionBootstrapUnavailableError) {
    return <SessionBootstrapError {...props} />
  }
  return <GeneralError />
}
