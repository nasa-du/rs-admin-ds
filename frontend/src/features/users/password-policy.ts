import type { RefinementCtx } from 'zod'
import type { TFunction } from 'i18next'

const MIN_TEMPORARY_PASSWORD_LENGTH = 15
const MAX_TEMPORARY_PASSWORD_LENGTH = 128

/** The server counts NFC-normalized Unicode code points (security policy). */
function temporaryPasswordLength(password: string): number {
  return Array.from(password.normalize('NFC')).length
}

type ManualPasswordValues = {
  passwordMode: 'generate' | 'manual'
  password: string
  confirmPassword: string
}

/**
 * Client-side subset of the password policy shared by the create and reset
 * flows: length in NFC code points plus confirmation match. The common-
 * password blocklist and identifier checks stay server-authoritative.
 */
export function validateManualPassword(
  values: ManualPasswordValues,
  ctx: RefinementCtx,
  t: TFunction<'validation'>
): void {
  if (values.passwordMode !== 'manual') return

  const length = temporaryPasswordLength(values.password)
  if (values.password === '') {
    ctx.addIssue({
      code: 'custom',
      path: ['password'],
      message: t('required'),
    })
  } else if (length < MIN_TEMPORARY_PASSWORD_LENGTH) {
    ctx.addIssue({
      code: 'custom',
      path: ['password'],
      message: t('min_length', { min: MIN_TEMPORARY_PASSWORD_LENGTH }),
    })
  } else if (length > MAX_TEMPORARY_PASSWORD_LENGTH) {
    ctx.addIssue({
      code: 'custom',
      path: ['password'],
      message: t('max_length', { max: MAX_TEMPORARY_PASSWORD_LENGTH }),
    })
  }

  if (values.confirmPassword !== values.password) {
    ctx.addIssue({
      code: 'custom',
      path: ['confirmPassword'],
      message: t('passwordMismatch'),
    })
  }
}
