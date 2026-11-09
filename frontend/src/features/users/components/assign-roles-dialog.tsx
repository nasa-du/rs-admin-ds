import { useMemo, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import {
  type ErrorTranslator,
  translateApiError,
  translateApiErrorFields,
} from '@/i18n/errors'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import type { UserDetail, UserSelfResponse } from '@/api/generated/model'
import { useSetUserRoles } from '@/api/generated/rs-admin'
import { markApiErrorHandled, toApiError } from '@/lib/api-error'
import { useRetryAfter } from '@/hooks/use-retry-after'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { ApiErrorAlert } from '@/components/api-error-alert'
import { runWithReauth } from '@/features/auth/reauth/reauth'
import { ReauthCancelledError } from '@/features/auth/reauth/reauth-controller'
import { isBaselineRole, isPrivilegedRole } from '../role-labels'
import { useRoleOptions } from '../use-role-options'
import {
  invalidateUserQueries,
  refreshUserDetailIfOutOfScope,
} from '../user-cache'
import { RoleMultiSelect } from './role-multi-select'

type AssignRolesDialogProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
  user: Pick<UserDetail, 'id' | 'username' | 'roles'>
  currentUser: UserSelfResponse | null
}

function extraRoleIds(user: Pick<UserDetail, 'roles'>): string[] {
  return user.roles
    .filter((role) => !isBaselineRole(role))
    .map((role) => role.id)
}

function sameIds(a: Set<string>, b: Set<string>): boolean {
  if (a.size !== b.size) return false
  for (const id of a) {
    if (!b.has(id)) return false
  }
  return true
}

/**
 * Role assignment dialog shared by the row action and the detail Sheet
 * (spec §9.8). The baseline `user` role is displayed locked and never enters
 * the submitted set; the server keeps it regardless.
 */
export function AssignRolesDialog({
  open,
  onOpenChange,
  user,
  currentUser,
}: AssignRolesDialogProps) {
  const { t } = useTranslation('users')
  const { t: tCommon } = useTranslation('common')
  const { t: tErrors } = useTranslation('errors')
  const queryClient = useQueryClient()

  const [roleIds, setRoleIds] = useState<string[]>(() => extraRoleIds(user))
  const [submitting, setSubmitting] = useState(false)
  const [submitError, setSubmitError] = useState<unknown>(null)
  const [formError, setFormError] = useState<string | null>(null)
  const [roleError, setRoleError] = useState<string | null>(null)
  const retryAfter = useRetryAfter(submitError)

  const roleOptionsQuery = useRoleOptions(open)

  const roleIdentityById = useMemo(() => {
    const map = new Map<
      string,
      { id: string; is_builtin: boolean; name: string }
    >()
    for (const role of user.roles) map.set(role.id, role)
    for (const role of roleOptionsQuery.data ?? []) map.set(role.id, role)
    return map
  }, [user.roles, roleOptionsQuery.data])

  const privilegedChanged = useMemo(() => {
    const initial = new Set(
      user.roles.filter(isPrivilegedRole).map((role) => role.id)
    )
    const current = new Set(
      roleIds.flatMap((id) => {
        const role = roleIdentityById.get(id)
        return role && isPrivilegedRole(role) ? [role.id] : []
      })
    )
    return !sameIds(initial, current)
  }, [user.roles, roleIds, roleIdentityById])

  const mutation = useSetUserRoles({
    mutation: { onError: markApiErrorHandled },
  })

  const isBusy = submitting || retryAfter > 0

  function handleOpenChange(next: boolean) {
    if (!next && isBusy) return
    onOpenChange(next)
  }

  async function onSubmit() {
    if (submitting) return
    setSubmitError(null)
    setFormError(null)
    setRoleError(null)
    setSubmitting(true)

    try {
      await runWithReauth(() =>
        mutation.mutateAsync({
          id: user.id,
          data: { role_ids: roleIds },
        })
      )

      await invalidateUserQueries(queryClient, user.id)
      toast.success(t('manage.assignRoles.success'))
      onOpenChange(false)
    } catch (error) {
      if (error instanceof ReauthCancelledError) return

      refreshUserDetailIfOutOfScope(queryClient, user.id, error)
      setSubmitError(error)
      mapServerError(error)
    } finally {
      setSubmitting(false)
    }
  }

  function mapServerError(error: unknown) {
    const fields = translateApiErrorFields(tErrors as ErrorTranslator, error)
    let nextRoleError: string | null = null
    let unmapped: string | null = null

    for (const { field, message } of fields) {
      if (field?.startsWith('roleIds')) {
        nextRoleError = message
      } else {
        unmapped ??= message
      }
    }

    const code = toApiError(error)?.code
    if (code === 'unknown_role' || code === 'permission_subset_violation') {
      nextRoleError ??= translateApiError(tErrors as ErrorTranslator, error)
    }

    if (nextRoleError) setRoleError(nextRoleError)
    if (unmapped) {
      setFormError(unmapped)
    } else if (!nextRoleError) {
      setFormError(translateApiError(tErrors as ErrorTranslator, error))
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent
        showCloseButton={!isBusy}
        className='max-h-[90vh] overflow-y-auto sm:max-w-2xl'
        onInteractOutside={(event) => {
          if (isBusy) event.preventDefault()
        }}
        onEscapeKeyDown={(event) => {
          if (isBusy) event.preventDefault()
        }}
      >
        <DialogHeader>
          <DialogTitle>{t('manage.assignRoles.title')}</DialogTitle>
          <DialogDescription>
            {t('manage.assignRoles.description', {
              username: user.username,
            })}
          </DialogDescription>
        </DialogHeader>

        <RoleMultiSelect
          roles={roleOptionsQuery.data ?? []}
          value={roleIds}
          onChange={setRoleIds}
          currentUser={currentUser}
          isLoading={roleOptionsQuery.isPending}
          isError={roleOptionsQuery.isError}
          onRetry={() => void roleOptionsQuery.refetch()}
          error={roleError}
          disabled={isBusy}
          privilegedWarning={
            privilegedChanged ? t('manage.assignRoles.privilegedChange') : null
          }
        />

        {formError && (
          <ApiErrorAlert
            error={submitError}
            message={formError}
            retryAfterSeconds={retryAfter}
          />
        )}

        <DialogFooter>
          <Button
            type='button'
            variant='outline'
            onClick={() => handleOpenChange(false)}
            disabled={isBusy}
          >
            {tCommon('actions.cancel')}
          </Button>
          <Button
            type='button'
            onClick={() => void onSubmit()}
            disabled={isBusy}
          >
            {submitting
              ? t('manage.assignRoles.submitting')
              : t('manage.assignRoles.submit')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
