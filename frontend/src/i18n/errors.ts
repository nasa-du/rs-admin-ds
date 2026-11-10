import { type FieldError, toApiError } from '@/lib/api-error'

/**
 * Static error-code → translation-key map.
 *
 * Keys must never be built at runtime from backend strings; unknown codes
 * fall back to `errors:common.unexpected`.
 */
export const ERROR_KEY_MAP = {
  network_error: 'errors:common.network_error',
  invalid_json: 'errors:common.invalid_json',
  unknown_field: 'errors:common.unknown_field',
  unsupported_media_type: 'errors:common.unsupported_media_type',
  method_not_allowed: 'errors:common.method_not_allowed',
  payload_too_large: 'errors:common.payload_too_large',
  validation_failed: 'errors:common.validation_failed',
  invalid_sort: 'errors:common.invalid_sort',
  page_size_too_large: 'errors:common.page_size_too_large',
  empty_update: 'errors:common.empty_update',
  invalid_time_range: 'errors:audit.invalid_time_range',
  time_range_too_large: 'errors:audit.time_range_too_large',
  invalid_credentials: 'errors:auth.invalid_credentials',
  access_token_invalid: 'errors:auth.access_token_invalid',
  session_invalid: 'errors:auth.session_invalid',
  session_expired: 'errors:auth.session_expired',
  refresh_token_invalid: 'errors:auth.refresh_token_invalid',
  refresh_reuse_detected: 'errors:auth.refresh_reuse_detected',
  account_disabled: 'errors:auth.account_disabled',
  temporary_password_expired: 'errors:auth.temporary_password_expired',
  login_locked: 'errors:auth.login_locked',
  rate_limited: 'errors:auth.rate_limited',
  password_change_required: 'errors:auth.password_change_required',
  csrf_failed: 'errors:auth.csrf_failed',
  reauth_required: 'errors:auth.reauth_required',
  invalid_current_password: 'errors:auth.invalid_current_password',
  password_policy_violation: 'errors:auth.password_policy_violation',
  password_unchanged: 'errors:auth.password_unchanged',
  forbidden: 'errors:common.forbidden',
  out_of_scope: 'errors:user.out_of_scope',
  self_action_forbidden: 'errors:user.self_action_forbidden',
  role_immutable: 'errors:role.role_immutable',
  permission_subset_violation: 'errors:role.permission_subset_violation',
  not_found: 'errors:common.not_found',
  identifier_taken: 'errors:user.identifier_taken',
  role_name_taken: 'errors:role.role_name_taken',
  role_in_use: 'errors:role.role_in_use',
  last_super_admin: 'errors:user.last_super_admin',
  unknown_role: 'errors:role.unknown_role',
  empty_permissions: 'errors:role.empty_permissions',
  invalid_permission: 'errors:role.invalid_permission',
  internal_error: 'errors:common.internal_error',
  service_unavailable: 'errors:common.service_unavailable',
} as const

type ErrorTranslationKey = (typeof ERROR_KEY_MAP)[keyof typeof ERROR_KEY_MAP]

const FALLBACK_ERROR_KEY = 'errors:common.unexpected' as const
const NETWORK_ERROR_KEY = 'errors:common.network_error' as const
const COMMON_RATE_LIMITED_KEY = 'errors:common.rate_limited' as const

export const PASSWORD_REASON_KEYS = {
  too_short: 'validation:password.too_short',
  too_long: 'validation:password.too_long',
  blocklisted: 'validation:password.blocklisted',
  equals_identifier: 'validation:password.equals_identifier',
} as const

type PasswordReasonKey =
  (typeof PASSWORD_REASON_KEYS)[keyof typeof PASSWORD_REASON_KEYS]

export const FIELD_CODE_KEYS = {
  required: 'validation:required',
  invalid: 'validation:invalid',
  invalid_format: 'validation:invalid_format',
  invalid_email: 'validation:invalid_email',
  invalid_url: 'validation:invalid_url',
  invalid_length: 'validation:invalid',
  min_length: 'validation:min_length',
  max_length: 'validation:max_length',
  identifier_taken: 'errors:user.identifier_taken',
} as const

type FieldTranslationKey =
  (typeof FIELD_CODE_KEYS)[keyof typeof FIELD_CODE_KEYS]

const FALLBACK_FIELD_KEY = 'validation:invalid' as const

export type ErrorTranslator = (
  key: string,
  options?: Record<string, unknown>
) => string

export type TranslateErrorOptions = {
  authContext?: boolean
  params?: Record<string, unknown>
}

function passwordReasonKey(reason: unknown): PasswordReasonKey | null {
  if (typeof reason !== 'string') return null
  return (
    PASSWORD_REASON_KEYS[reason as keyof typeof PASSWORD_REASON_KEYS] ?? null
  )
}

export function translateErrorCode(
  t: ErrorTranslator,
  code: string | undefined,
  options: TranslateErrorOptions = {}
): string {
  if (!code) return t(FALLBACK_ERROR_KEY)

  if (code === 'rate_limited' && !options.authContext) {
    return t(COMMON_RATE_LIMITED_KEY)
  }

  if (code === 'password_policy_violation') {
    const reasonKey = passwordReasonKey(options.params?.reason)
    if (reasonKey) return t(reasonKey)
  }

  const key = (ERROR_KEY_MAP as Record<string, ErrorTranslationKey>)[code]
  return t(key ?? FALLBACK_ERROR_KEY)
}

export function translateApiError(
  t: ErrorTranslator,
  error: unknown,
  options: TranslateErrorOptions = {}
): string {
  const apiError = toApiError(error)
  if (!apiError) {
    return t(
      error instanceof TypeError ? NETWORK_ERROR_KEY : FALLBACK_ERROR_KEY
    )
  }

  return translateErrorCode(t, apiError.code, options)
}

const FIELD_ROOT_NAMES = new Set([
  'identifier',
  'password',
  'current_password',
  'new_password',
  'username',
  'email',
  'display_name',
  'phone',
  'bio',
  'urls',
  'name',
  'description',
  'dob',
  'language',
  'items',
  'type',
  'mobile',
  'communication_emails',
  'marketing_emails',
  'social_emails',
  'security_emails',
  'role_ids',
  'permissions',
  'comment',
  'reason',
  'page',
  'page_size',
  'sort',
])

function toFieldSegment(segment: string): string {
  return segment
    .split('_')
    .map((part, index) =>
      index === 0 ? part : part.charAt(0).toUpperCase() + part.slice(1)
    )
    .join('')
}

/**
 * Convert a JSON pointer from the API into a React Hook Form field name.
 * Unknown roots return `null`, which the caller should treat as a form-level
 * error.
 */
export function pointerToFieldName(pointer: string): string | null {
  const segments = pointer
    .split('/')
    .filter(Boolean)
    .map((segment) => decodeURIComponent(segment))

  const [root, ...rest] = segments
  if (!root || !FIELD_ROOT_NAMES.has(root)) return null

  return [root, ...rest].map(toFieldSegment).join('.')
}

export type TranslatedFieldError = {
  field: string | null
  message: string
}

export function translateFieldError(
  t: ErrorTranslator,
  fieldError: FieldError
): TranslatedFieldError {
  const key: FieldTranslationKey | PasswordReasonKey =
    fieldError.code === 'password_policy_violation'
      ? (passwordReasonKey(fieldError.params?.reason) ?? FALLBACK_FIELD_KEY)
      : ((FIELD_CODE_KEYS as Record<string, FieldTranslationKey>)[
          fieldError.code
        ] ?? FALLBACK_FIELD_KEY)

  return {
    field: pointerToFieldName(fieldError.pointer),
    message: t(key, fieldError.params ?? {}),
  }
}

export function translateApiErrorFields(
  t: ErrorTranslator,
  error: unknown
): TranslatedFieldError[] {
  const apiError = toApiError(error)
  if (!apiError) return []
  return apiError.fieldErrors.map((fieldError) =>
    translateFieldError(t, fieldError)
  )
}
