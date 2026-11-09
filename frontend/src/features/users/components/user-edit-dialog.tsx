import { useEffect, useMemo, useState } from 'react'
import { z } from 'zod'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { useQueryClient } from '@tanstack/react-query'
import {
  type ErrorTranslator,
  translateApiError,
  translateApiErrorFields,
} from '@/i18n/errors'
import type { TFunction } from 'i18next'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import type { UserSelfResponse, UserSummary } from '@/api/generated/model'
import {
  getGetUserQueryKey,
  getListUsersQueryKey,
  useUpdateUser,
} from '@/api/generated/rs-admin'
import { ApiError, toApiError } from '@/lib/api-error'
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
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form'
import { Input } from '@/components/ui/input'
import { ApiErrorAlert } from '@/components/api-error-alert'
import { ConfirmDialog } from '@/components/confirm-dialog'
import { runWithReauth } from '@/features/auth/reauth/reauth'
import { ReauthCancelledError } from '@/features/auth/reauth/reauth-controller'
import { buildUserUpdate, updateTouchesIdentifiers } from '../users-form'

function makeEditUserSchema(t: TFunction<'validation'>) {
  return z.object({
    displayName: z.string().trim().min(1, t('required')),
    username: z.string().trim().min(1, t('required')),
    email: z.string().trim().min(1, t('required')).email(t('invalid_email')),
    phone: z.string(),
  })
}

type EditUserFormValues = z.infer<ReturnType<typeof makeEditUserSchema>>

const EDITABLE_FIELDS = new Set(['displayName', 'username', 'email', 'phone'])

function toFormValues(user: UserSummary): EditUserFormValues {
  return {
    displayName: user.display_name,
    username: user.username,
    email: user.email,
    phone: user.phone ?? '',
  }
}

type UserEditDialogProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
  user: UserSummary
  currentUser: UserSelfResponse | null
}

/**
 * Edit-user dialog (spec §9.8): prefilled from the list row, only dirty
 * fields enter the PATCH body, identifier changes run behind `runWithReauth`,
 * and closing a dirty form requires confirmation.
 */
export function UserEditDialog({
  open,
  onOpenChange,
  user,
  currentUser,
}: UserEditDialogProps) {
  const { t } = useTranslation('users')
  const { t: tCommon } = useTranslation('common')
  const { t: tValidation } = useTranslation('validation')
  const { t: tErrors } = useTranslation('errors')
  const queryClient = useQueryClient()
  const schema = useMemo(() => makeEditUserSchema(tValidation), [tValidation])

  const [submitting, setSubmitting] = useState(false)
  const [submitError, setSubmitError] = useState<unknown>(null)
  const [formError, setFormError] = useState<string | null>(null)
  const [discardOpen, setDiscardOpen] = useState(false)
  const retryAfter = useRetryAfter(submitError)

  const form = useForm<EditUserFormValues>({
    resolver: zodResolver(schema),
    defaultValues: toFormValues(user),
  })

  const updateMutation = useUpdateUser({
    mutation: {
      onError: (error) => {
        const normalized = toApiError(error)
        if (normalized instanceof ApiError) normalized.handled = true
      },
    },
  })

  useEffect(() => {
    if (!open) return
    form.reset(toFormValues(user))
    setSubmitError(null)
    setFormError(null)
    setDiscardOpen(false)
    // Reset when the dialog opens for a different user; including the form
    // object would reset on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, user.id])

  const values = form.watch()
  const update = buildUserUpdate(user, values)
  const identifiersDirty = updateTouchesIdentifiers(update)
  const isSelf = currentUser?.id === user.id
  const isBusy = submitting || retryAfter > 0

  function handleOpenChange(next: boolean) {
    if (next) {
      onOpenChange(true)
      return
    }
    if (isBusy) return
    if (update) {
      setDiscardOpen(true)
      return
    }
    onOpenChange(false)
  }

  function discardAndClose() {
    form.reset(toFormValues(user))
    setDiscardOpen(false)
    onOpenChange(false)
  }

  function mapServerError(error: unknown) {
    const normalized = toApiError(error)
    if (normalized?.code === 'out_of_scope') {
      // The target may have moved out of scope since the row was rendered.
      // Invalidate the detail so the sheet re-reads the server's answer.
      void queryClient.invalidateQueries({
        queryKey: getGetUserQueryKey(user.id),
      })
    }

    const fields = translateApiErrorFields(tErrors as ErrorTranslator, error)
    let mapped = false

    for (const { field, message } of fields) {
      if (field && EDITABLE_FIELDS.has(field)) {
        form.setError(field as keyof EditUserFormValues, {
          type: 'server',
          message,
        })
        mapped = true
      }
    }

    if (!mapped) {
      setFormError(translateApiError(tErrors as ErrorTranslator, error))
    }
  }

  async function onSubmit(formValues: EditUserFormValues) {
    if (submitting) return

    const requestBody = buildUserUpdate(user, formValues)
    if (!requestBody) return

    if (isSelf) {
      setFormError(t('edit.self'))
      return
    }

    setSubmitError(null)
    setFormError(null)
    form.clearErrors()
    setSubmitting(true)

    const request = () =>
      updateMutation.mutateAsync({ id: user.id, data: requestBody })

    try {
      if (updateTouchesIdentifiers(requestBody)) {
        await runWithReauth(request)
      } else {
        await request()
      }

      await Promise.all([
        queryClient.invalidateQueries({ queryKey: getListUsersQueryKey() }),
        queryClient.invalidateQueries({
          queryKey: getGetUserQueryKey(user.id),
        }),
      ])
      toast.success(t('edit.success'))
      onOpenChange(false)
    } catch (error) {
      if (error instanceof ReauthCancelledError) return

      setSubmitError(error)
      mapServerError(error)
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <>
      <Dialog open={open} onOpenChange={handleOpenChange}>
        <DialogContent
          showCloseButton={!isBusy}
          onInteractOutside={(event) => {
            if (isBusy) event.preventDefault()
          }}
          onEscapeKeyDown={(event) => {
            if (isBusy) event.preventDefault()
          }}
        >
          <DialogHeader>
            <DialogTitle>{t('edit.title')}</DialogTitle>
            <DialogDescription>
              {t('edit.description', { username: user.username })}
            </DialogDescription>
          </DialogHeader>

          <Form {...form}>
            <form
              onSubmit={form.handleSubmit(onSubmit)}
              className='grid gap-4'
              noValidate
            >
              {isSelf && (
                <div
                  role='alert'
                  className='rounded-md border border-destructive/50 bg-destructive/5 p-3 text-sm text-destructive'
                >
                  {t('edit.self')}
                </div>
              )}

              <div className='grid gap-4 sm:grid-cols-2'>
                <FormField
                  control={form.control}
                  name='displayName'
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t('edit.displayName')}</FormLabel>
                      <FormControl>
                        <Input autoComplete='off' {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name='phone'
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t('edit.phone')}</FormLabel>
                      <FormControl>
                        <Input type='tel' autoComplete='off' {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name='username'
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t('edit.username')}</FormLabel>
                      <FormControl>
                        <Input autoComplete='off' {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name='email'
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t('edit.email')}</FormLabel>
                      <FormControl>
                        <Input type='email' autoComplete='off' {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>

              {identifiersDirty && (
                <p className='rounded-md border bg-muted/50 p-3 text-sm text-muted-foreground'>
                  {t('edit.identifierHint')}
                </p>
              )}

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
                <Button type='submit' disabled={!update || isBusy || isSelf}>
                  {submitting ? t('edit.submitting') : t('edit.submit')}
                </Button>
              </DialogFooter>
            </form>
          </Form>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={discardOpen}
        onOpenChange={setDiscardOpen}
        title={t('edit.discardTitle')}
        desc={t('edit.discardDescription')}
        confirmText={t('edit.discard')}
        destructive
        handleConfirm={discardAndClose}
      />
    </>
  )
}
