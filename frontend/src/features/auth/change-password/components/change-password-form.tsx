import { useMemo, useState } from 'react'
import { z } from 'zod'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import {
  type ErrorTranslator,
  translateApiError,
  translateApiErrorFields,
} from '@/i18n/errors'
import { type TFunction } from 'i18next'
import { KeyRound } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { useChangePassword } from '@/api/generated/rs-admin'
import { ApiError, toApiError } from '@/lib/api-error'
import { changePasswordRedirectTarget } from '@/lib/redirect'
import { cn } from '@/lib/utils'
import { useRetryAfter } from '@/hooks/use-retry-after'
import { Button } from '@/components/ui/button'
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form'
import { ApiErrorAlert } from '@/components/api-error-alert'
import { PasswordInput } from '@/components/password-input'
import { authKeys, authSession } from '@/features/auth/auth-session'

const MIN_PASSWORD_LENGTH = 15
const MAX_PASSWORD_LENGTH = 128

function makeChangePasswordSchema(t: TFunction<'validation'>) {
  return z
    .object({
      currentPassword: z.string().min(1, t('required')),
      newPassword: z
        .string()
        .min(1, t('required'))
        .min(MIN_PASSWORD_LENGTH, t('min_length', { min: MIN_PASSWORD_LENGTH }))
        .max(
          MAX_PASSWORD_LENGTH,
          t('max_length', { max: MAX_PASSWORD_LENGTH })
        ),
      confirmPassword: z.string().min(1, t('required')),
    })
    .refine((values) => values.newPassword === values.confirmPassword, {
      path: ['confirmPassword'],
      message: t('passwordMismatch'),
    })
}

type ChangePasswordFormValues = z.infer<
  ReturnType<typeof makeChangePasswordSchema>
>

type ChangePasswordFormProps = React.HTMLAttributes<HTMLFormElement> & {
  redirect?: string
  mustChangePassword?: boolean
}

export function ChangePasswordForm({
  className,
  redirect,
  mustChangePassword = false,
  ...props
}: ChangePasswordFormProps) {
  const { t } = useTranslation('auth')
  const { t: tValidation } = useTranslation('validation')
  const { t: tErrors } = useTranslation('errors')
  const schema = useMemo(
    () => makeChangePasswordSchema(tValidation),
    [tValidation]
  )
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const [submitError, setSubmitError] = useState<unknown>(null)
  const [formError, setFormError] = useState<string | null>(null)
  const retryAfter = useRetryAfter(submitError)

  const form = useForm<ChangePasswordFormValues>({
    resolver: zodResolver(schema),
    defaultValues: {
      currentPassword: '',
      newPassword: '',
      confirmPassword: '',
    },
  })

  const changePasswordMutation = useChangePassword({
    mutation: {
      // Errors are rendered inline by this form, not as a toast.
      onError: (error) => {
        const normalized = toApiError(error)
        if (normalized instanceof ApiError) normalized.handled = true
      },
    },
  })

  async function onSubmit(values: ChangePasswordFormValues) {
    setSubmitError(null)
    setFormError(null)
    form.clearErrors()

    try {
      const response = await changePasswordMutation.mutateAsync({
        data: {
          current_password: values.currentPassword,
          new_password: values.newPassword,
        },
      })

      // Keep the current device signed in with the rotated token, then
      // refresh `/me` so the forced-change guard sees the cleared flag.
      authSession.applyAccessToken(response.access_token)
      try {
        await authSession.refreshMe()
      } catch {
        // The password change succeeded; only the profile refresh failed.
        // Drop the stale copy so the route guard re-fetches it.
        queryClient.removeQueries({ queryKey: authKeys.me })
      }
      toast.success(t('changePassword.success'))
      await navigate({
        href: changePasswordRedirectTarget(redirect),
        replace: true,
      })
    } catch (error) {
      setSubmitError(error)
      showError(error)
    }
  }

  function showError(error: unknown) {
    const normalized = toApiError(error)
    const fields = normalized
      ? translateApiErrorFields(tErrors as ErrorTranslator, normalized)
      : []

    let mapped = false
    for (const { field, message } of fields) {
      if (
        field === 'currentPassword' ||
        field === 'newPassword' ||
        field === 'confirmPassword'
      ) {
        form.setError(field, { type: 'server', message })
        mapped = true
      }
    }
    if (mapped) return

    const message = translateApiError(tErrors as ErrorTranslator, error)

    if (normalized?.code === 'invalid_current_password') {
      form.setError('currentPassword', { type: 'server', message })
      return
    }
    if (
      normalized?.code === 'password_unchanged' ||
      normalized?.code === 'password_policy_violation'
    ) {
      form.setError('newPassword', { type: 'server', message })
      return
    }

    setFormError(fields[0]?.message ?? message)
  }

  const isBusy = changePasswordMutation.isPending || retryAfter > 0

  return (
    <Form {...form}>
      <form
        onSubmit={form.handleSubmit(onSubmit)}
        className={cn('grid gap-4', className)}
        noValidate
        {...props}
      >
        {mustChangePassword && (
          <div
            role='status'
            className='grid gap-1 rounded-md border bg-muted/50 p-3 text-sm'
          >
            <p className='font-medium'>{t('changePassword.forcedTitle')}</p>
            <p className='text-muted-foreground'>
              {t('changePassword.forcedDescription')}
            </p>
          </div>
        )}

        <FormField
          control={form.control}
          name='currentPassword'
          render={({ field }) => (
            <FormItem>
              <FormLabel>{t('changePassword.currentPassword')}</FormLabel>
              <FormControl>
                <PasswordInput autoComplete='current-password' {...field} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name='newPassword'
          render={({ field }) => (
            <FormItem>
              <FormLabel>{t('changePassword.newPassword')}</FormLabel>
              <FormControl>
                <PasswordInput autoComplete='new-password' {...field} />
              </FormControl>
              <FormDescription>
                {t('changePassword.newPasswordHint')}
              </FormDescription>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name='confirmPassword'
          render={({ field }) => (
            <FormItem>
              <FormLabel>{t('changePassword.confirmPassword')}</FormLabel>
              <FormControl>
                <PasswordInput autoComplete='new-password' {...field} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />

        {formError && (
          <ApiErrorAlert
            error={submitError}
            message={formError}
            retryAfterSeconds={retryAfter}
          />
        )}

        <Button className='mt-2' type='submit' disabled={isBusy}>
          <KeyRound />
          {changePasswordMutation.isPending
            ? t('changePassword.submitting')
            : t('changePassword.submit')}
        </Button>
      </form>
    </Form>
  )
}
