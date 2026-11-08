import { useTranslation } from 'react-i18next'
import { toApiError } from '@/lib/api-error'
import { RequestId } from './request-id'

type ApiErrorAlertProps = {
  error: unknown
  message: string
  /** Seconds remaining on a 429 `Retry-After` countdown. */
  retryAfterSeconds?: number
}

/**
 * Inline form-level API error (spec §9.12): mapped message, optional
 * countdown, technical code and a copyable `request_id`.
 */
export function ApiErrorAlert({
  error,
  message,
  retryAfterSeconds = 0,
}: ApiErrorAlertProps) {
  const { t } = useTranslation('common')
  const apiError = toApiError(error)

  return (
    <div
      role='alert'
      className='grid gap-1 rounded-md border border-destructive/50 bg-destructive/5 p-3 text-sm text-destructive'
    >
      <p>{message}</p>
      {retryAfterSeconds > 0 && (
        <p>{t('error.retryIn', { seconds: retryAfterSeconds })}</p>
      )}
      {apiError && (
        <p className='text-xs opacity-80'>
          {t('error.technicalCode', { code: apiError.code })}
        </p>
      )}
      <RequestId requestId={apiError?.requestId} />
    </div>
  )
}
