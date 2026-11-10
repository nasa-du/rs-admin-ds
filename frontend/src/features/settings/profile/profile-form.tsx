import { useEffect, useId, useMemo, useState } from 'react'
import { z } from 'zod'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { useQueryClient } from '@tanstack/react-query'
import {
  type ErrorTranslator,
  translateApiError,
  translateApiErrorFields,
} from '@/i18n/errors'
import { type TFunction } from 'i18next'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import type { UserSelfResponse } from '@/api/generated/model'
import { useUpdateMe } from '@/api/generated/rs-admin'
import { ApiError, toApiError } from '@/lib/api-error'
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
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { ApiErrorAlert } from '@/components/api-error-alert'
import { authKeys } from '@/features/auth/auth-session'
import { useAuth } from '@/features/auth/use-auth'
import { UserRoleChips } from '@/features/users/components/user-role-chips'
import { UserStatusBadges } from '@/features/users/components/user-status-badges'

const DISPLAY_NAME_MAX = 64

function makeProfileFormSchema(t: TFunction<'validation'>) {
  return z.object({
    // The server counts Unicode code points, so the client must too; `.max()`
    // would count UTF-16 code units and reject valid 64-emoji names.
    displayName: z
      .string()
      .trim()
      .superRefine((value, context) => {
        const length = Array.from(value).length
        if (length === 0) {
          context.addIssue({ code: 'custom', message: t('required') })
        } else if (length > DISPLAY_NAME_MAX) {
          context.addIssue({
            code: 'custom',
            message: t('max_length', { max: DISPLAY_NAME_MAX }),
          })
        }
      }),
  })
}

type ProfileFormValues = z.infer<ReturnType<typeof makeProfileFormSchema>>

type ReadOnlyFieldProps = {
  label: string
  value: string
  emptyLabel?: string
}

/**
 * Administrator-managed profile fields: visible, never editable here.
 * Self-service editing is limited to `display_name` (spec §9.11).
 */
function ReadOnlyField({ label, value, emptyLabel }: ReadOnlyFieldProps) {
  const id = useId()
  return (
    <div className='grid gap-2'>
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        readOnly
        aria-readonly='true'
        value={value}
        placeholder={emptyLabel}
        className='bg-muted'
      />
    </div>
  )
}

function AdminManagedHint() {
  const { t } = useTranslation('settings')
  return (
    <p className='text-xs text-muted-foreground'>{t('profile.adminManaged')}</p>
  )
}

export function ProfileForm() {
  const { t } = useTranslation('settings')
  const { t: tValidation } = useTranslation('validation')
  const { t: tErrors } = useTranslation('errors')
  const user = useAuth()
  const queryClient = useQueryClient()
  const schema = useMemo(
    () => makeProfileFormSchema(tValidation),
    [tValidation]
  )

  const [submitError, setSubmitError] = useState<unknown>(null)
  const [formError, setFormError] = useState<string | null>(null)
  const retryAfter = useRetryAfter(submitError)

  const form = useForm<ProfileFormValues>({
    resolver: zodResolver(schema),
    defaultValues: { displayName: user?.display_name ?? '' },
    mode: 'onChange',
  })

  const updateMutation = useUpdateMe({
    mutation: {
      // Errors are rendered inline by this form, not as a toast.
      onError: (error) => {
        const normalized = toApiError(error)
        if (normalized instanceof ApiError) normalized.handled = true
      },
    },
  })

  useEffect(() => {
    form.reset({ displayName: user?.display_name ?? '' })
  }, [form, user?.display_name])

  if (!user) return null

  function showError(error: unknown) {
    const fields = translateApiErrorFields(tErrors as ErrorTranslator, error)
    const displayNameError = fields.find(
      (field) => field.field === 'displayName'
    )
    if (displayNameError) {
      form.setError('displayName', {
        type: 'server',
        message: displayNameError.message,
      })
      return
    }

    setFormError(translateApiError(tErrors as ErrorTranslator, error))
  }

  async function onSubmit(values: ProfileFormValues) {
    setSubmitError(null)
    setFormError(null)
    form.clearErrors()

    try {
      const updated: UserSelfResponse = await updateMutation.mutateAsync({
        data: { display_name: values.displayName },
      })
      queryClient.setQueryData(authKeys.me, updated)
      form.reset({ displayName: updated.display_name })
      toast.success(t('profile.saved'))
    } catch (error) {
      setSubmitError(error)
      showError(error)
    }
  }

  const isBusy = updateMutation.isPending || retryAfter > 0

  return (
    <Form {...form}>
      <form
        onSubmit={form.handleSubmit(onSubmit)}
        className='space-y-8'
        noValidate
      >
        <FormField
          control={form.control}
          name='displayName'
          render={({ field }) => (
            <FormItem>
              <FormLabel>{t('profile.displayName')}</FormLabel>
              <FormControl>
                <Input autoComplete='nickname' {...field} />
              </FormControl>
              <FormDescription>{t('profile.displayNameHint')}</FormDescription>
              <FormMessage />
            </FormItem>
          )}
        />

        <div className='space-y-4'>
          <h3 className='text-base font-medium'>
            {t('profile.identitySection')}
          </h3>
          <div className='grid gap-4'>
            <div className='grid gap-1.5'>
              <ReadOnlyField
                label={t('profile.username')}
                value={user.username}
              />
              <AdminManagedHint />
            </div>
            <div className='grid gap-1.5'>
              <ReadOnlyField label={t('profile.email')} value={user.email} />
              <AdminManagedHint />
            </div>
            <div className='grid gap-1.5'>
              <ReadOnlyField
                label={t('profile.phone')}
                value={user.phone ?? ''}
                emptyLabel={t('profile.phoneEmpty')}
              />
              <AdminManagedHint />
            </div>
          </div>
        </div>

        <div className='grid gap-4 sm:grid-cols-2'>
          <div className='grid gap-2'>
            <Label>{t('profile.status')}</Label>
            <UserStatusBadges
              status={user.status}
              mustChangePassword={user.must_change_password}
            />
          </div>
          <div className='grid gap-2'>
            <Label>{t('profile.roles')}</Label>
            <UserRoleChips roles={user.roles} />
          </div>
        </div>

        {formError && (
          <ApiErrorAlert
            error={submitError}
            message={formError}
            retryAfterSeconds={retryAfter}
          />
        )}

        <Button type='submit' disabled={isBusy}>
          {updateMutation.isPending
            ? t('profile.submitting')
            : t('profile.submit')}
        </Button>
      </form>
    </Form>
  )
}
