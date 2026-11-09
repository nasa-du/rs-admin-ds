import type { UpdateUserRequest, UserSummary } from '@/api/generated/model'

/**
 * Pure normalization and dirty-field rules for the edit dialog (spec §9.8).
 * Only changed fields enter the PATCH body; clearing the phone sends an
 * explicit `null`, and an unchanged form never produces the 422
 * `empty_update` request.
 */

export type UserEditValues = {
  displayName: string
  username: string
  email: string
  phone: string
}

export function normalizeUsername(value: string): string {
  return value.trim().toLowerCase()
}

export function normalizeEmail(value: string): string {
  return value.normalize('NFC').trim().toLowerCase()
}

export function buildUserUpdate(
  user: UserSummary,
  values: UserEditValues
): UpdateUserRequest | null {
  const displayName = values.displayName.trim()
  const username = normalizeUsername(values.username)
  const email = normalizeEmail(values.email)
  const phone = values.phone.trim()

  const update: UpdateUserRequest = {}

  if (displayName !== user.display_name) {
    update.display_name = displayName
  }
  if (username !== user.username) {
    update.username = username
  }
  if (email !== user.email) {
    update.email = email
  }

  const currentPhone = user.phone ?? null
  if (phone === '') {
    if (currentPhone !== null) update.phone = null
  } else if (phone !== currentPhone) {
    update.phone = phone
  }

  return Object.keys(update).length > 0 ? update : null
}

export function updateTouchesIdentifiers(
  update: UpdateUserRequest | null
): boolean {
  return Boolean(
    update && (update.username !== undefined || update.email !== undefined)
  )
}
