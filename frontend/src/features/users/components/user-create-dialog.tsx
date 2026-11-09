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
import type {
  CreateUserRequest,
  RoleSummary,
  UserCreatedResponse,
  UserSelfResponse,
} from '@/api/generated/model'
import { getListUsersQueryKey, useCreateUser } from '@/api/generated/rs-admin'
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
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { ApiErrorAlert } from '@/components/api-error-alert'
import { ConfirmDialog } from '@/components/confirm-dialog'
import { PasswordInput } from '@/components/password-input'
import { runWithReauth } from '@/features/auth/reauth/reauth'
import { ReauthCancelledError } from '@/features/auth/reauth/reauth-controller'
import { validateManualPassword } from '../password-policy'
import { normalizeEmail, normalizeUsername } from '../users-form'
import { RoleMultiSelect } from './role-multi-select'
import { TemporaryPasswordPanel } from './temporary-password-panel'

const DEFAULT_VALUES = {
  username: '',
  email: '',
  displayName: '',
  phone: '',
  passwordMode: 'generate' as const,
  password: '',
  confirmPassword: '',
  roleIds: [] as string[],
}

function makeCreateUserSchema(t: TFunction<'validation'>) {
  return z
    .object({
      username: z.string().trim().min(1, t('required')),
      email: z.string().trim().min(1, t('required')).email(t('invalid_email')),
      displayName: z.string(),
      phone: z.string(),
      passwordMode: z.enum(['generate', 'manual']),
      password: z.string(),
      confirmPassword: z.string(),
      roleIds: z.array(z.string()),
    })
    .superRefine((values, ctx) => validateManualPassword(values, ctx, t))
}

type CreateUserFormValues = z.infer<ReturnType<typeof makeCreateUserSchema>>

const FORM_FIELDS = new Set([
  'username',
  'email',
  'displayName',
  'phone',
  'password',
  'confirmPassword',
])

type UserCreateDialogProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
  canAssignRoles: boolean
  currentUser: UserSelfResponse | null
  roleOptions: RoleSummary[]
  rolesLoading?: boolean
  rolesError?: boolean
  onRetryRoles?: () => void
}

/**
 * Create-user dialog (spec §9.8): generated or manual temporary password,
 * optional extra roles, mandatory step-up, and a one-time password reveal
 * that is dropped from memory as soon as the dialog closes.
 */
export function UserCreateDialog({
  open,
  onOpenChange,
  canAssignRoles,
  currentUser,
  roleOptions,
  rolesLoading = false,
  rolesError = false,
  onRetryRoles,
}: UserCreateDialogProps) {
  const { t } = useTranslation('users')
  const { t: tCommon } = useTranslation('common')
  const { t: tValidation } = useTranslation('validation')
  const { t: tErrors } = useTranslation('errors')
  const queryClient = useQueryClient()
  const schema = useMemo(() => makeCreateUserSchema(tValidation), [tValidation])

  const [created, setCreated] = useState<UserCreatedResponse | null>(null)
  const [submitting, setSubmitting] = useState(false)
  const [submitError, setSubmitError] = useState<unknown>(null)
  const [formError, setFormError] = useState<string | null>(null)
  const [roleError, setRoleError] = useState<string | null>(null)
  const [confirmCloseOpen, setConfirmCloseOpen] = useState(false)
  const retryAfter = useRetryAfter(submitError)

  const form = useForm<CreateUserFormValues>({
    resolver: zodResolver(schema),
    defaultValues: DEFAULT_VALUES,
  })
  const passwordMode = form.watch('passwordMode')
  const roleIds = form.watch('roleIds')

  const createMutation = useCreateUser({
    mutation: {
      // The response carries the one-time password; do not retain it after the
      // dialog closes.
      gcTime: 0,
      // Server errors render inline; keep the global mutation toast out of it.
      onError: (error) => {
        const normalized = toApiError(error)
        if (normalized instanceof ApiError) normalized.handled = true
      },
    },
  })

  useEffect(() => {
    if (open) return
    setCreated(null)
    setSubmitError(null)
    setFormError(null)
    setRoleError(null)
    setConfirmCloseOpen(false)
    form.reset(DEFAULT_VALUES)
    createMutation.reset()
    // Reset only on close; form and mutation identities are stable enough and
    // including them would re-run the effect on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  function closeAndReset() {
    setCreated(null)
    setSubmitError(null)
    setFormError(null)
    setRoleError(null)
    setConfirmCloseOpen(false)
    form.reset(DEFAULT_VALUES)
    createMutation.reset()
    onOpenChange(false)
  }

  function handleOpenChange(next: boolean) {
    if (next) {
      onOpenChange(true)
      return
    }
    if (submitting) return
    if (created) {
      setConfirmCloseOpen(true)
      return
    }
    closeAndReset()
  }

  function mapServerError(error: unknown): boolean {
    const fields = translateApiErrorFields(tErrors as ErrorTranslator, error)
    let mapped = false
    let nextRoleError: string | null = null

    for (const { field, message } of fields) {
      if (field && FORM_FIELDS.has(field)) {
        form.setError(field as keyof CreateUserFormValues, {
          type: 'server',
          message,
        })
        mapped = true
      } else if (canAssignRoles && field && field.startsWith('roleIds')) {
        nextRoleError = message
        mapped = true
      }
    }

    if (nextRoleError) setRoleError(nextRoleError)

    const normalized = toApiError(error)
    if (
      canAssignRoles &&
      (normalized?.code === 'unknown_role' ||
        normalized?.code === 'permission_subset_violation')
    ) {
      setRoleError(
        nextRoleError ?? translateApiError(tErrors as ErrorTranslator, error)
      )
      return true
    }

    return mapped
  }

  async function onSubmit(values: CreateUserFormValues) {
    if (submitting) return
    setSubmitError(null)
    setFormError(null)
    setRoleError(null)
    form.clearErrors()
    setSubmitting(true)

    const data: CreateUserRequest = {
      username: normalizeUsername(values.username),
      email: normalizeEmail(values.email),
    }

    const displayName = values.displayName.trim()
    if (displayName) data.display_name = displayName

    const phone = values.phone.trim()
    if (phone) data.phone = phone

    if (values.passwordMode === 'manual') {
      data.password = values.password
    }
    if (canAssignRoles && values.roleIds.length > 0) {
      data.role_ids = values.roleIds
    }

    try {
      const response = await runWithReauth(() =>
        createMutation.mutateAsync({ data })
      )

      setCreated(response)
      await queryClient.invalidateQueries({
        queryKey: getListUsersQueryKey(),
      })
      toast.success(t('create.success'))
    } catch (error) {
      if (error instanceof ReauthCancelledError) return

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
          className='max-h-[90vh] overflow-y-auto sm:max-w-2xl'
          onInteractOutside={(event) => {
            if (submitting) event.preventDefault()
          }}
          onEscapeKeyDown={(event) => {
            if (submitting) event.preventDefault()
          }}
        >
          <DialogHeader>
            <DialogTitle>
              {created ? t('password.createdTitle') : t('create.title')}
            </DialogTitle>
            <DialogDescription>
              {created
                ? t('password.createdDescription', {
                    username: created.user.username,
                  })
                : t('create.description')}
            </DialogDescription>
          </DialogHeader>

          {created ? (
            <>
              <TemporaryPasswordPanel
                password={created.temporary_password}
                expiresAt={created.temporary_password_expires_at}
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
                <div className='grid gap-4 sm:grid-cols-2'>
                  <FormField
                    control={form.control}
                    name='username'
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>{t('create.username')}</FormLabel>
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
                        <FormLabel>{t('create.email')}</FormLabel>
                        <FormControl>
                          <Input type='email' autoComplete='off' {...field} />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                  <FormField
                    control={form.control}
                    name='displayName'
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>{t('create.displayName')}</FormLabel>
                        <FormControl>
                          <Input autoComplete='off' {...field} />
                        </FormControl>
                        <FormDescription>
                          {t('create.displayNameHint')}
                        </FormDescription>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                  <FormField
                    control={form.control}
                    name='phone'
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>{t('create.phone')}</FormLabel>
                        <FormControl>
                          <Input type='tel' autoComplete='off' {...field} />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                </div>

                <FormField
                  control={form.control}
                  name='passwordMode'
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t('create.passwordMode')}</FormLabel>
                      <FormControl>
                        <RadioGroup
                          value={field.value}
                          onValueChange={field.onChange}
                          className='grid gap-2 sm:grid-cols-2'
                        >
                          <div className='flex items-start gap-2 rounded-md border p-3'>
                            <RadioGroupItem
                              value='generate'
                              id='create-password-generate'
                              aria-label={t('create.generate')}
                            />
                            <Label
                              htmlFor='create-password-generate'
                              className='grid gap-1 font-normal'
                            >
                              <span>{t('create.generate')}</span>
                              <span className='text-xs text-muted-foreground'>
                                {t('create.generateHint')}
                              </span>
                            </Label>
                          </div>
                          <div className='flex items-start gap-2 rounded-md border p-3'>
                            <RadioGroupItem
                              value='manual'
                              id='create-password-manual'
                              aria-label={t('create.manual')}
                            />
                            <Label
                              htmlFor='create-password-manual'
                              className='grid gap-1 font-normal'
                            >
                              <span>{t('create.manual')}</span>
                              <span className='text-xs text-muted-foreground'>
                                {t('create.manualHint')}
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
                          <FormLabel>{t('create.password')}</FormLabel>
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
                          <FormLabel>{t('create.confirmPassword')}</FormLabel>
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

                {canAssignRoles && (
                  <RoleMultiSelect
                    roles={roleOptions}
                    value={roleIds}
                    onChange={(next) =>
                      form.setValue('roleIds', next, { shouldDirty: true })
                    }
                    currentUser={currentUser}
                    isLoading={rolesLoading}
                    isError={rolesError}
                    onRetry={onRetryRoles}
                    error={roleError}
                    disabled={isBusy}
                  />
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
                    {submitting ? t('create.submitting') : t('create.submit')}
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
