import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { type ErrorTranslator, translateApiError } from '@/i18n/errors'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import type {
  RoleDetail,
  RoleSummary,
  UserSelfResponse,
} from '@/api/generated/model'
import { getListRolesQueryKey, useDeleteRole } from '@/api/generated/rs-admin'
import { markApiErrorHandled, toApiError } from '@/lib/api-error'
import { hasPermission } from '@/lib/permissions'
import { roleDisplayName } from '@/lib/role-labels'
import { Button } from '@/components/ui/button'
import { ApiErrorAlert } from '@/components/api-error-alert'
import { ConfirmDialog } from '@/components/confirm-dialog'
import { runWithReauth } from '@/features/auth/reauth/reauth'
import { ReauthCancelledError } from '@/features/auth/reauth/reauth-controller'

type RoleDeleteDialogProps = {
  open: boolean
  role: RoleSummary | RoleDetail
  currentUser: UserSelfResponse | null
  onOpenChange: (open: boolean) => void
  onDeleted: () => void
}

/**
 * Role soft-delete confirmation (spec §9.9). Deletion always runs behind
 * `runWithReauth`; a 409 `role_in_use` stays in the dialog and offers the
 * filtered user list when the operator can read users.
 */
export function RoleDeleteDialog({
  open,
  role,
  currentUser,
  onOpenChange,
  onDeleted,
}: RoleDeleteDialogProps) {
  const { t } = useTranslation('roles')
  const { t: tErrors } = useTranslation('errors')
  const { t: tCommon } = useTranslation('common')
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  const [submitting, setSubmitting] = useState(false)
  const [submitError, setSubmitError] = useState<unknown>(null)

  const deleteMutation = useDeleteRole({
    mutation: { onError: markApiErrorHandled },
  })

  const canViewUsers = hasPermission(currentUser, 'users.read')
  const errorCode = toApiError(submitError)?.code
  const isInUse = errorCode === 'role_in_use'

  function handleOpenChange(next: boolean) {
    if (!next && submitting) return
    if (!next) setSubmitError(null)
    onOpenChange(next)
  }

  async function handleConfirm() {
    if (submitting) return

    setSubmitError(null)
    setSubmitting(true)

    try {
      await runWithReauth(() => deleteMutation.mutateAsync({ id: role.id }))
      await queryClient.invalidateQueries({
        queryKey: getListRolesQueryKey(),
      })
      toast.success(t('delete.success'))
      onDeleted()
    } catch (error) {
      if (error instanceof ReauthCancelledError) return
      setSubmitError(error)
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <ConfirmDialog
      open={open}
      onOpenChange={handleOpenChange}
      title={t('delete.title')}
      desc={
        <div className='grid gap-2'>
          <p>
            {t('delete.description', {
              name: roleDisplayName(t, role),
            })}
          </p>
          <p>
            {t('list.permissionCount', { count: role.permission_keys.length })}
          </p>
          <p>{t('delete.warning')}</p>
          {isInUse && (
            <div
              role='alert'
              className='mt-2 grid gap-2 rounded-md border border-destructive/50 bg-destructive/5 p-3 text-destructive'
            >
              <p className='font-medium'>{t('delete.inUseTitle')}</p>
              <p>{t('delete.inUseDescription')}</p>
              <p className='text-xs opacity-80'>
                {tCommon('error.technicalCode', { code: errorCode })}
              </p>
              {canViewUsers && (
                <Button
                  type='button'
                  variant='outline'
                  size='sm'
                  className='justify-self-start'
                  onClick={() =>
                    void navigate({
                      to: '/users',
                      search: { role_id: role.id },
                    })
                  }
                >
                  {t('delete.viewUsers')}
                </Button>
              )}
            </div>
          )}
          {submitError != null && !isInUse && (
            <ApiErrorAlert
              error={submitError}
              message={translateApiError(
                tErrors as ErrorTranslator,
                submitError
              )}
            />
          )}
        </div>
      }
      confirmText={t('delete.confirm')}
      destructive
      isLoading={submitting}
      handleConfirm={() => void handleConfirm()}
    />
  )
}
