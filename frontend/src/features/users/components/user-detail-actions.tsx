import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { type ErrorTranslator, translateApiError } from '@/i18n/errors'
import {
  Ban,
  KeyRound,
  LogOut,
  Pencil,
  Power,
  Shield,
  Unlock,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import type { UserDetail, UserSelfResponse } from '@/api/generated/model'
import {
  useRevokeUserSessions,
  useSetUserStatus,
  useUnlockUser,
} from '@/api/generated/rs-admin'
import { markApiErrorHandled } from '@/lib/api-error'
import { hasPermission } from '@/lib/permissions'
import { useRetryAfter } from '@/hooks/use-retry-after'
import { Button } from '@/components/ui/button'
import { SheetFooter } from '@/components/ui/sheet'
import { ApiErrorAlert } from '@/components/api-error-alert'
import { ConfirmDialog } from '@/components/confirm-dialog'
import { DisabledTooltip } from '@/components/disabled-tooltip'
import { runWithReauth } from '@/features/auth/reauth/reauth'
import { ReauthCancelledError } from '@/features/auth/reauth/reauth-controller'
import { isAccountLocked } from '../account-status'
import {
  invalidateUserQueries,
  refreshUserDetailIfOutOfScope,
} from '../user-cache'
import { AssignRolesDialog } from './assign-roles-dialog'
import { ResetPasswordDialog } from './reset-password-dialog'

type UserDetailActionsProps = {
  user: UserDetail
  currentUser: UserSelfResponse | null
  canEdit: boolean
  onEdit: (user: UserDetail) => void
}

type ActionPending = 'status' | 'unlock' | 'revoke'

/**
 * Management actions for the detail Sheet (spec §9.8): entries are hidden
 * without the matching action permission and disabled with a tooltip for the
 * current user. Successful actions keep the Sheet open and re-read the detail.
 */
export function UserDetailActions({
  user,
  currentUser,
  canEdit,
  onEdit,
}: UserDetailActionsProps) {
  const { t } = useTranslation('users')
  const { t: tErrors } = useTranslation('errors')
  const queryClient = useQueryClient()

  const [actionPending, setActionPending] = useState<ActionPending | null>(null)
  const [actionError, setActionError] = useState<unknown>(null)
  const [disableOpen, setDisableOpen] = useState(false)
  const [disableError, setDisableError] = useState<unknown>(null)
  const [revokeOpen, setRevokeOpen] = useState(false)
  const [revokeError, setRevokeError] = useState<unknown>(null)
  const [resetOpen, setResetOpen] = useState(false)
  const [assignOpen, setAssignOpen] = useState(false)

  const statusMutation = useSetUserStatus({
    mutation: { onError: markApiErrorHandled },
  })
  const unlockMutation = useUnlockUser({
    mutation: { onError: markApiErrorHandled },
  })
  const revokeMutation = useRevokeUserSessions({
    mutation: { onError: markApiErrorHandled },
  })

  const actionRetryAfter = useRetryAfter(actionError)
  const disableRetryAfter = useRetryAfter(disableError)
  const revokeRetryAfter = useRetryAfter(revokeError)

  const isSelf = currentUser?.id === user.id
  const canAssignRoles =
    hasPermission(currentUser, 'users.assign_role') &&
    hasPermission(currentUser, 'roles.read')
  const canResetPassword = hasPermission(currentUser, 'users.reset_password')
  const canSetStatus = hasPermission(currentUser, 'users.set_status')
  const canUnlock = hasPermission(currentUser, 'users.unlock')
  const canRevokeSessions = hasPermission(currentUser, 'sessions.revoke')
  const showUnlock = canUnlock && isAccountLocked(user.locked_until)

  const hasActions =
    canEdit ||
    canAssignRoles ||
    canResetPassword ||
    canSetStatus ||
    showUnlock ||
    canRevokeSessions

  if (!hasActions) return null

  const selfReason = isSelf ? t('edit.self') : undefined
  const busy = actionPending !== null || actionRetryAfter > 0

  function errorMessage(error: unknown): string {
    return translateApiError(tErrors as ErrorTranslator, error)
  }

  async function runAction<T>({
    pending,
    operation,
    successMessage,
    setError,
    onSuccess,
  }: {
    pending: ActionPending
    operation: () => Promise<T>
    successMessage: string
    setError: (error: unknown) => void
    onSuccess?: () => void
  }) {
    if (actionPending) return
    setActionPending(pending)
    try {
      await operation()
      await invalidateUserQueries(queryClient, user.id)
      toast.success(successMessage)
      onSuccess?.()
    } catch (error) {
      if (error instanceof ReauthCancelledError) return
      refreshUserDetailIfOutOfScope(queryClient, user.id, error)
      setError(error)
    } finally {
      setActionPending(null)
    }
  }

  function enableUser() {
    setActionError(null)
    return runAction({
      pending: 'status',
      operation: () =>
        runWithReauth(() =>
          statusMutation.mutateAsync({
            id: user.id,
            data: { status: 'active' },
          })
        ),
      successMessage: t('manage.status.enableSuccess'),
      setError: setActionError,
    })
  }

  function disableUser() {
    setDisableError(null)
    return runAction({
      pending: 'status',
      operation: () =>
        runWithReauth(() =>
          statusMutation.mutateAsync({
            id: user.id,
            data: { status: 'disabled' },
          })
        ),
      successMessage: t('manage.status.disableSuccess'),
      setError: setDisableError,
      onSuccess: () => setDisableOpen(false),
    })
  }

  function unlockUser() {
    setActionError(null)
    return runAction({
      pending: 'unlock',
      operation: () => unlockMutation.mutateAsync({ id: user.id }),
      successMessage: t('manage.unlock.success'),
      setError: setActionError,
    })
  }

  function revokeSessions() {
    setRevokeError(null)
    return runAction({
      pending: 'revoke',
      operation: () =>
        runWithReauth(() => revokeMutation.mutateAsync({ id: user.id })),
      successMessage: t('manage.revokeSessions.success'),
      setError: setRevokeError,
      onSuccess: () => setRevokeOpen(false),
    })
  }

  return (
    <>
      <SheetFooter>
        <div className='grid gap-2 sm:grid-cols-2'>
          {canEdit && (
            <ActionButton
              icon={Pencil}
              label={t('detail.edit')}
              disabledReason={selfReason}
              disabled={busy}
              onClick={() => onEdit(user)}
            />
          )}
          {canAssignRoles && (
            <ActionButton
              icon={Shield}
              label={t('manage.assignRoles.action')}
              disabledReason={selfReason}
              disabled={busy}
              onClick={() => setAssignOpen(true)}
            />
          )}
          {canResetPassword && (
            <ActionButton
              icon={KeyRound}
              label={t('manage.resetPassword.action')}
              disabledReason={selfReason}
              disabled={busy}
              onClick={() => setResetOpen(true)}
            />
          )}
          {showUnlock && (
            <ActionButton
              icon={Unlock}
              label={t('manage.unlock.action')}
              disabledReason={selfReason}
              pending={actionPending === 'unlock'}
              disabled={busy}
              onClick={() => void unlockUser()}
            />
          )}
          {canSetStatus &&
            (user.status === 'active' ? (
              <ActionButton
                icon={Ban}
                label={t('manage.status.disableAction')}
                disabledReason={selfReason}
                disabled={busy}
                onClick={() => {
                  setDisableError(null)
                  setDisableOpen(true)
                }}
              />
            ) : (
              <ActionButton
                icon={Power}
                label={t('manage.status.enableAction')}
                disabledReason={selfReason}
                pending={actionPending === 'status'}
                disabled={busy}
                onClick={() => void enableUser()}
              />
            ))}
          {canRevokeSessions && (
            <ActionButton
              icon={LogOut}
              label={t('manage.revokeSessions.action')}
              disabledReason={selfReason}
              disabled={busy}
              onClick={() => {
                setRevokeError(null)
                setRevokeOpen(true)
              }}
            />
          )}
        </div>

        {actionError != null && (
          <ApiErrorAlert
            error={actionError}
            message={errorMessage(actionError)}
            retryAfterSeconds={actionRetryAfter}
          />
        )}
      </SheetFooter>

      {resetOpen && (
        <ResetPasswordDialog open onOpenChange={setResetOpen} user={user} />
      )}

      {assignOpen && (
        <AssignRolesDialog
          open
          onOpenChange={setAssignOpen}
          user={user}
          currentUser={currentUser}
        />
      )}

      <ConfirmDialog
        open={disableOpen}
        onOpenChange={(next) => {
          if (!next && actionPending) return
          setDisableOpen(next)
          if (!next) setDisableError(null)
        }}
        title={t('manage.status.disableTitle', { username: user.username })}
        desc={t('manage.status.disableDescription')}
        confirmText={t('manage.status.disableConfirm')}
        destructive
        isLoading={actionPending === 'status' || disableRetryAfter > 0}
        handleConfirm={() => void disableUser()}
      >
        {disableError != null && (
          <ApiErrorAlert
            error={disableError}
            message={errorMessage(disableError)}
            retryAfterSeconds={disableRetryAfter}
          />
        )}
      </ConfirmDialog>

      <ConfirmDialog
        open={revokeOpen}
        onOpenChange={(next) => {
          if (!next && actionPending) return
          setRevokeOpen(next)
          if (!next) setRevokeError(null)
        }}
        title={t('manage.revokeSessions.title', { username: user.username })}
        desc={t('manage.revokeSessions.description')}
        confirmText={t('manage.revokeSessions.confirm')}
        destructive
        isLoading={actionPending === 'revoke' || revokeRetryAfter > 0}
        handleConfirm={() => void revokeSessions()}
      >
        {revokeError != null && (
          <ApiErrorAlert
            error={revokeError}
            message={errorMessage(revokeError)}
            retryAfterSeconds={revokeRetryAfter}
          />
        )}
      </ConfirmDialog>
    </>
  )
}

type ActionButtonProps = {
  icon: React.ComponentType<{ className?: string }>
  label: string
  onClick: () => void
  disabledReason?: string
  disabled?: boolean
  pending?: boolean
}

function ActionButton({
  icon: Icon,
  label,
  onClick,
  disabledReason,
  disabled = false,
  pending = false,
}: ActionButtonProps) {
  const button = (
    <Button
      type='button'
      variant='outline'
      className='w-full justify-start gap-2'
      disabled={disabled || pending || Boolean(disabledReason)}
      onClick={onClick}
    >
      <Icon className='size-4' />
      {label}
    </Button>
  )

  return <DisabledTooltip reason={disabledReason}>{button}</DisabledTooltip>
}
