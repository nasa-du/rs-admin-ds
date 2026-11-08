import i18n from '@/i18n'
import { translateApiError, type ErrorTranslator } from '@/i18n/errors'
import { toast } from 'sonner'
import { toApiError } from './api-error'

export function handleServerError(error: unknown) {
  if (import.meta.env.DEV) {
    // eslint-disable-next-line no-console
    console.log(error)
  }

  if (toApiError(error)?.handled) return

  const t = i18n.t.bind(i18n) as ErrorTranslator
  toast.error(translateApiError(t, error))
}
