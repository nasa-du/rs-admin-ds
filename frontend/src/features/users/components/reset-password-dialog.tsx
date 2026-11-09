import { useMemo, useState } from 'react'
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
import type {
  ResetUserPasswordRequest,
  UserCreatedResponse,
  UserDetail,
} from '@/api/generated/model'
import { useResetUserPassword } from '@/api/generated/rs-admin'
import { markApiErrorHandled } from '@/lib/api-error'
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
import { Label } from '@/components/ui/label'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { ApiErrorAlert } from '@/components/api-error-alert'
import { ConfirmDialog } from '@/components/confirm-dialog'
import { PasswordInput } from '@/components/password-input'
import { runWithReauth } from '@/features/auth/reauth/reauth'
import { ReauthCancelledError } from '@/features/auth/reauth/reauth-controller'
import { validateManualPassword } from '../password-policy'
import {
  invalidateUserQueries,
  refreshUserDetailIfOutOfScope,
} from '../user-cache'
import { TemporaryPasswordPanel } from './temporary-password-panel'

const DEFAULT_VALUES = {
  passwordMode: 'generate' as const,
  password: '',
  confirmPassword: '',
}

function makeResetPasswordSchema(t: TFunction<'validation'>) {
  return z
    .object({
      passwordMode: z.enum(['generate', 'manual']),
      password: z.string(),
      confirmPassword: z.string(),
    })
    .superRefine((values, ctx) => validateManualPassword(values, ctx, t))
}

type ResetPasswordFormValues = z.infer<
  ReturnType<typeof makeResetPasswordSchema>
>

type ResetPasswordDialogProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
  user: Pick<UserDetail, 'id' | 'username'>
}

/**
 * Credential reset (spec §9.8): generated or manual temporary password behind
 * reauthentication, followed by the shared one-time reveal panel. The plaintext
 * password lives only in this component's state and is dropped on close.
 */
export function ResetPasswordDialog({
  open,
  onOpenChange,
  user,
}: ResetPasswordDialogProps) {
  const { t } = useTranslation('users')
  const { t: tCommon } = useTranslation('common')
  const { t: tValidation } = useTranslation('validation')
  const { t: tErrors } = useTranslation('errors')
  const queryClient = useQueryClient()
  const schema = useMemo(
    () => makeResetPasswordSchema(tValidation),
    [tValidation]
  )

  const [issued, setIssued] = useState<UserCreatedResponse | null>(null)
  const [submitting, setSubmitting] = useState(false)
  const [submitError, setSubmitError] = useState<unknown>(null)
  const [formError, setFormError] = useState<string | null>(null)
  const [confirmCloseOpen, setConfirmCloseOpen] = useState(false)
  const retryAfter = useRetryAfter(submitError)

  const form = useForm<ResetPasswordFormValues>({
    resolver: zodResolver(schema),
    defaultValues: DEFAULT_VALUES,
  })
  const passwordMode = form.watch('passwordMode')

  const resetMutation = useResetUserPassword({
    mutation: {
      // The response carries the one-time password; do not retain it.
      gcTime: 0,
      onError: markApiErrorHandled,
    },
  })

  function closeAndReset() {
    setIssued(null)
    setSubmitError(null)
    setFormError(null)
    setConfirmCloseOpen(false)
    form.reset(DEFAULT_VALUES)
    resetMutation.reset()
    onOpenChange(false)
  }

  function handleOpenChange(next: boolean) {
    if (next) {
      onOpenChange(true)
      return
    }
    if (submitting) return
    if (issued) {
      setConfirmCloseOpen(true)
      return
    }
    closeAndReset()
  }

  function mapServerError(error: unknown): boolean {
    const fields = translateApiErrorFields(tErrors as ErrorTranslator, error)
    let surfaced = false
    let unmapped: string | null = null

    for (const { field, message } of fields) {
      if (field === 'password') {
        form.setError('password', { type: 'server', message })
        surfaced = true
      } else {
        unmapped ??= message
      }
    }

    // Unknown pointers belong at the form level (spec §9.8).
    if (unmapped) {
      setFormError(unmapped)
      surfaced = true
    }

    return surfaced
  }

  async function onSubmit(values: ResetPasswordFormValues) {
    if (submitting) return
    setSubmitError(null)
    setFormError(null)
    form.clearErrors()
    setSubmitting(true)

    const data: ResetUserPasswordRequest =
      values.passwordMode === 'manual' ? { password: values.password } : {}

    try {
      const response = await runWithReauth(() =>
        resetMutation.mutateAsync({ id: user.id, data })
      )

      setIssued(response)
      await invalidateUserQueries(queryClient, user.id)
      toast.success(t('manage.resetPassword.success'))
    } catch (error) {
      if (error instanceof ReauthCancelledError) return

      refreshUserDetailIfOutOfScope(queryClient, user.id, error)
      setSubmitError(error)
      if (!mapServerError(error)) {
        setFormError(translateApiError(tErrors as ErrorTranslator, error))
      }
    } finally {
      setSubmitting(false)
    }
  }

  const isBusy = submitting || retryAfter > 0

  return (
    <>
      <Dialog open={open} onOpenChange={handleOpenChange}>
        <DialogContent
          showCloseButton={!submitting}
          className='max-h-[90vh] overflow-y-auto sm:max-w-xl'
          onInteractOutside={(event) => {
            if (submitting) event.preventDefault()
          }}
          onEscapeKeyDown={(event) => {
            if (submitting) event.preventDefault()
          }}
        >
          <DialogHeader>
            <DialogTitle>
              {issued
                ? t('manage.resetPassword.issuedTitle')
                : t('manage.resetPassword.title')}
            </DialogTitle>
            <DialogDescription>
              {issued
                ? t('manage.resetPassword.issuedDescription', {
                    username: user.username,
                  })
                : t('manage.resetPassword.description', {
                    username: user.username,
                  })}
            </DialogDescription>
          </DialogHeader>

          {issued ? (
            <>
              <TemporaryPasswordPanel
                password={issued.temporary_password}
                expiresAt={issued.temporary_password_expires_at}
              />
              <DialogFooter>
                <Button type='button' onClick={closeAndReset}>
                  {t('password.saved')}
                </Button>
              </DialogFooter>
            </>
          ) : (
            <Form {...form}>
              <form
                onSubmit={form.handleSubmit(onSubmit)}
                className='grid gap-4'
                noValidate
              >
                <p className='rounded-md border bg-muted/50 p-3 text-sm text-muted-foreground'>
                  {t('manage.resetPassword.consequences')}
                </p>

                <FormField
                  control={form.control}
                  name='passwordMode'
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        {t('manage.resetPassword.passwordMode')}
                      </FormLabel>
                      <FormControl>
                        <RadioGroup
                          value={field.value}
                          onValueChange={field.onChange}
                          className='grid gap-2 sm:grid-cols-2'
                        >
                          <div className='flex items-start gap-2 rounded-md border p-3'>
                            <RadioGroupItem
                              value='generate'
                              id='reset-password-generate'
                              aria-label={t('manage.resetPassword.generate')}
                            />
                            <Label
                              htmlFor='reset-password-generate'
                              className='grid gap-1 font-normal'
                            >
                              <span>{t('manage.resetPassword.generate')}</span>
                              <span className='text-xs text-muted-foreground'>
                                {t('manage.resetPassword.generateHint')}
                              </span>
                            </Label>
                          </div>
                          <div className='flex items-start gap-2 rounded-md border p-3'>
                            <RadioGroupItem
                              value='manual'
                              id='reset-password-manual'
                              aria-label={t('manage.resetPassword.manual')}
                            />
                            <Label
                              htmlFor='reset-password-manual'
                              className='grid gap-1 font-normal'
                            >
                              <span>{t('manage.resetPassword.manual')}</span>
                              <span className='text-xs text-muted-foreground'>
                                {t('manage.resetPassword.manualHint')}
                              </span>
                            </Label>
                          </div>
                        </RadioGroup>
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                {passwordMode === 'manual' && (
                  <div className='grid gap-4 sm:grid-cols-2'>
                    <FormField
                      control={form.control}
                      name='password'
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>
                            {t('manage.resetPassword.password')}
                          </FormLabel>
                          <FormControl>
                            <PasswordInput
                              autoComplete='new-password'
                              {...field}
                            />
                          </FormControl>
                          <FormMessage />
                        </FormItem>
                      )}
                    />
                    <FormField
                      control={form.control}
                      name='confirmPassword'
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>
                            {t('manage.resetPassword.confirmPassword')}
                          </FormLabel>
                          <FormControl>
                            <PasswordInput
                              autoComplete='new-password'
                              {...field}
                            />
                          </FormControl>
                          <FormMessage />
                        </FormItem>
                      )}
                    />
                  </div>
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
                  <Button type='submit' disabled={isBusy}>
                    {submitting
                      ? t('manage.resetPassword.submitting')
                      : t('manage.resetPassword.submit')}
                  </Button>
                </DialogFooter>
              </form>
            </Form>
          )}
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={confirmCloseOpen}
        onOpenChange={setConfirmCloseOpen}
        title={t('password.closeTitle')}
        desc={t('password.closeDescription')}
        confirmText={t('password.closeConfirm')}
        handleConfirm={closeAndReset}
      />
    </>
  )
}
