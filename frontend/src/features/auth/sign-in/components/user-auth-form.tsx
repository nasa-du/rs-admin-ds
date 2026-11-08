import { useMemo, useState } from 'react'
import { z } from 'zod'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { useNavigate } from '@tanstack/react-router'
import {
  type ErrorTranslator,
  translateApiError,
  translateApiErrorFields,
} from '@/i18n/errors'
import { type TFunction } from 'i18next'
import { LogIn } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { useLogin } from '@/api/generated/rs-admin'
import { ApiError, toApiError } from '@/lib/api-error'
import { changePasswordRedirectTarget, sanitizeRedirect } from '@/lib/redirect'
import { cn } from '@/lib/utils'
import { useRetryAfter } from '@/hooks/use-retry-after'
import { Button } from '@/components/ui/button'
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
import { PasswordInput } from '@/components/password-input'
import { authSession } from '@/features/auth/auth-session'

function makeSignInSchema(t: TFunction<'validation'>) {
  return z.object({
    identifier: z.string().min(1, t('required')),
    password: z.string().min(1, t('required')),
  })
}

type SignInFormValues = z.infer<ReturnType<typeof makeSignInSchema>>

type UserAuthFormProps = React.HTMLAttributes<HTMLFormElement> & {
  /** Sanitized again before navigation; raw search params are untrusted. */
  redirect?: string
}

export function UserAuthForm({
  className,
  redirect,
  ...props
}: UserAuthFormProps) {
  const { t } = useTranslation('auth')
  const { t: tValidation } = useTranslation('validation')
  const { t: tErrors } = useTranslation('errors')
  const schema = useMemo(() => makeSignInSchema(tValidation), [tValidation])
  const navigate = useNavigate()

  const [submitError, setSubmitError] = useState<unknown>(null)
  const [formError, setFormError] = useState<string | null>(null)
  const retryAfter = useRetryAfter(submitError)

  const form = useForm<SignInFormValues>({
    resolver: zodResolver(schema),
    defaultValues: {
      identifier: '',
      password: '',
    },
  })

  const loginMutation = useLogin({
    mutation: {
      // Errors are rendered inline by this form, not as a toast.
      onError: (error) => {
        const normalized = toApiError(error)
        if (normalized instanceof ApiError) normalized.handled = true
      },
    },
  })

  async function onSubmit(values: SignInFormValues) {
    setSubmitError(null)
    setFormError(null)
    form.clearErrors()

    try {
      const response = await loginMutation.mutateAsync({ data: values })
      authSession.applyLogin(response)

      const target = sanitizeRedirect(redirect)
      if (response.user.must_change_password) {
        await navigate({
          to: '/change-password',
          search: { redirect: changePasswordRedirectTarget(redirect) },
          replace: true,
        })
        return
      }

      await navigate({ href: target, replace: true })
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
      if (field === 'identifier' || field === 'password') {
        form.setError(field, { type: 'server', message })
        mapped = true
      }
    }

    if (mapped) return
    if (fields[0]) {
      setFormError(fields[0].message)
      return
    }

    setFormError(
      translateApiError(tErrors as ErrorTranslator, error, {
        authContext: true,
      })
    )
  }

  const isBusy = loginMutation.isPending || retryAfter > 0

  return (
    <Form {...form}>
      <form
        onSubmit={form.handleSubmit(onSubmit)}
        className={cn('grid gap-3', className)}
        noValidate
        {...props}
      >
        <FormField
          control={form.control}
          name='identifier'
          render={({ field }) => (
            <FormItem>
              <FormLabel>{t('signIn.identifier')}</FormLabel>
              <FormControl>
                <Input
                  autoComplete='username'
                  placeholder={t('signIn.identifierPlaceholder')}
                  {...field}
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name='password'
          render={({ field }) => (
            <FormItem className='relative'>
              <FormLabel>{t('signIn.password')}</FormLabel>
              <FormControl>
                <PasswordInput
                  autoComplete='current-password'
                  placeholder={t('signIn.passwordPlaceholder')}
                  {...field}
                />
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
          <LogIn />
          {loginMutation.isPending
            ? t('signIn.submitting')
            : t('signIn.submit')}
        </Button>
      </form>
    </Form>
  )
}
