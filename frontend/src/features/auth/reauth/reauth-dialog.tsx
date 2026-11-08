import { useEffect, useMemo, useSyncExternalStore } from 'react'
import { z } from 'zod'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { type ErrorTranslator, translateApiError } from '@/i18n/errors'
import type { TFunction } from 'i18next'
import { useTranslation } from 'react-i18next'
import { toApiError } from '@/lib/api-error'
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
import { ApiErrorAlert } from '@/components/api-error-alert'
import { PasswordInput } from '@/components/password-input'
import { reauthController } from './reauth'
import { type ReauthController } from './reauth-controller'

function makeReauthSchema(t: TFunction<'validation'>) {
  return z.object({
    password: z.string().min(1, t('required')),
  })
}

type ReauthFormValues = z.infer<ReturnType<typeof makeReauthSchema>>

type ReauthDialogProps = {
  controller?: ReauthController
}

function isInvalidCurrentPassword(error: unknown): boolean {
  return toApiError(error)?.code === 'invalid_current_password'
}

/**
 * The single, app-wide reauthentication prompt (ADR-0006 §7). It is driven by
 * a `ReauthController`, so concurrent reauthentication failures share one
 * dialog and one `POST /auth/reauthenticate`.
 */
export function ReauthDialog({
  controller = reauthController,
}: ReauthDialogProps) {
  const { t } = useTranslation('auth')
  const { t: tCommon } = useTranslation('common')
  const { t: tValidation } = useTranslation('validation')
  const { t: tErrors } = useTranslation('errors')
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot
  )
  const retryAfter = useRetryAfter(state.error)
  const schema = useMemo(() => makeReauthSchema(tValidation), [tValidation])

  const form = useForm<ReauthFormValues>({
    resolver: zodResolver(schema),
    defaultValues: { password: '' },
  })

  // Every prompt starts empty; errors from the previous attempt stay in the
  // controller until the next submission.
  useEffect(() => {
    if (state.open) form.reset({ password: '' })
  }, [state.open, form])

  const passwordError = isInvalidCurrentPassword(state.error)
  const isBusy = state.submitting || retryAfter > 0

  async function onSubmit(values: ReauthFormValues) {
    try {
      await controller.submitPassword(values.password)
    } catch (error) {
      if (isInvalidCurrentPassword(error)) {
        form.setError('password', {
          type: 'server',
          message: translateApiError(tErrors as ErrorTranslator, error),
        })
      }
    }
  }

  function handleOpenChange(open: boolean) {
    if (open || state.submitting) return
    controller.cancel()
  }

  return (
    <Dialog open={state.open} onOpenChange={handleOpenChange}>
      <DialogContent showCloseButton={!state.submitting}>
        <DialogHeader>
          <DialogTitle>{t('reauth.title')}</DialogTitle>
          <DialogDescription>{t('reauth.description')}</DialogDescription>
        </DialogHeader>

        <Form {...form}>
          <form
            onSubmit={form.handleSubmit(onSubmit)}
            className='grid gap-4'
            noValidate
          >
            <FormField
              control={form.control}
              name='password'
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{t('reauth.password')}</FormLabel>
                  <FormControl>
                    <PasswordInput
                      autoComplete='current-password'
                      autoFocus
                      {...field}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            {state.error != null && !passwordError && (
              <ApiErrorAlert
                error={state.error}
                message={translateApiError(
                  tErrors as ErrorTranslator,
                  state.error,
                  {
                    authContext: true,
                  }
                )}
                retryAfterSeconds={retryAfter}
              />
            )}

            <DialogFooter>
              <Button
                type='button'
                variant='outline'
                onClick={() => controller.cancel()}
                disabled={state.submitting}
              >
                {tCommon('actions.cancel')}
              </Button>
              <Button type='submit' disabled={isBusy}>
                {state.submitting ? t('reauth.submitting') : t('reauth.submit')}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}

type ReauthProviderProps = {
  children: React.ReactNode
  controller?: ReauthController
}

/** Mounts the app-wide dialog next to the routed application. */
export function ReauthProvider({
  children,
  controller = reauthController,
}: ReauthProviderProps) {
  return (
    <>
      {children}
      <ReauthDialog controller={controller} />
    </>
  )
}
