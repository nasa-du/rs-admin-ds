import { useMemo, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { type ErrorTranslator } from '@/i18n/errors'
import { ArrowLeft, AlertTriangle } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import {
  getListRolesQueryKey,
  useCreateRole,
  useListPermissions,
} from '@/api/generated/rs-admin'
import { markApiErrorHandled } from '@/lib/api-error'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Skeleton } from '@/components/ui/skeleton'
import { Textarea } from '@/components/ui/textarea'
import { ApiErrorAlert } from '@/components/api-error-alert'
import { ConfigDrawer } from '@/components/config-drawer'
import { Header } from '@/components/layout/header'
import { Main } from '@/components/layout/main'
import { ProfileDropdown } from '@/components/profile-dropdown'
import { Search } from '@/components/search'
import { ThemeSwitch } from '@/components/theme-switch'
import { runWithReauth } from '@/features/auth/reauth/reauth'
import { ReauthCancelledError } from '@/features/auth/reauth/reauth-controller'
import { PermissionMatrix } from './components/permission-matrix'
import { orderedPermissionKeys } from './permission-catalog'
import { mapRoleServerError } from './role-errors'
import {
  ROLE_DESCRIPTION_MAX_LENGTH,
  ROLE_NAME_MAX_LENGTH,
  buildCreateRoleRequest,
  makeRoleMetadataSchema,
  makeRolePermissionsSchema,
} from './role-form'
import { UnsavedChangesDialog } from './unsaved-changes'
import { useUnsavedChangesGuard } from './use-unsaved-changes'

/**
 * Create-role page (spec §9.9): one POST with metadata and permission keys,
 * the full permission matrix, step-up behind `runWithReauth`, and dirty
 * navigation protection.
 */
export function RoleCreate() {
  const { t } = useTranslation('roles')
  const { t: tCommon } = useTranslation('common')
  const { t: tErrors } = useTranslation('errors')
  const { t: tValidation } = useTranslation('validation')
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [nameError, setNameError] = useState<string | null>(null)
  const [descriptionError, setDescriptionError] = useState<string | null>(null)
  const [permissionError, setPermissionError] = useState<string | null>(null)
  const [formError, setFormError] = useState<string | null>(null)
  const [submitError, setSubmitError] = useState<unknown>(null)
  const [submitting, setSubmitting] = useState(false)

  const metadataSchema = useMemo(
    () => makeRoleMetadataSchema(tValidation),
    [tValidation]
  )
  const permissionsSchema = useMemo(
    () => makeRolePermissionsSchema(tValidation),
    [tValidation]
  )

  const permissionsQuery = useListPermissions()
  const groups = useMemo(
    () => permissionsQuery.data?.groups ?? [],
    [permissionsQuery.data]
  )

  const dirty =
    name.trim() !== '' || description.trim() !== '' || selected.size > 0
  const { blocker, allowNextNavigation } = useUnsavedChangesGuard(dirty)

  const createMutation = useCreateRole({
    mutation: {
      onError: markApiErrorHandled,
    },
  })

  function goBack() {
    void navigate({ to: '/roles' })
  }

  function mapServerError(error: unknown) {
    const mapping = mapRoleServerError(tErrors as ErrorTranslator, error)

    switch (mapping.kind) {
      case 'name':
        setNameError(mapping.message)
        break
      case 'description':
        setDescriptionError(mapping.message)
        break
      case 'permissions':
        setPermissionError(mapping.message)
        break
      default:
        setFormError(mapping.message)
    }
  }

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (submitting) return

    setNameError(null)
    setDescriptionError(null)
    setPermissionError(null)
    setFormError(null)
    setSubmitError(null)

    const metadata = metadataSchema.safeParse({ name, description })
    const permissions = permissionsSchema.safeParse({
      permissionKeys: [...selected],
    })
    let invalid = false

    if (!metadata.success) {
      const issue = metadata.error.issues[0]
      if (issue?.path[0] === 'description') {
        setDescriptionError(issue.message)
      } else {
        setNameError(issue?.message ?? tValidation('invalid'))
      }
      invalid = true
    }
    if (!permissions.success) {
      setPermissionError(permissions.error.issues[0]?.message ?? null)
      invalid = true
    }
    if (invalid || !metadata.success) return

    setSubmitting(true)

    try {
      const created = await runWithReauth(() =>
        createMutation.mutateAsync({
          data: buildCreateRoleRequest(
            metadata.data,
            orderedPermissionKeys(groups, selected)
          ),
        })
      )

      await queryClient.invalidateQueries({
        queryKey: getListRolesQueryKey(),
      })
      toast.success(t('new.success'))
      allowNextNavigation()
      void navigate({
        to: '/roles/$roleId',
        params: { roleId: created.id },
      })
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
      <Header fixed>
        <Search className='me-auto' />
        <ThemeSwitch />
        <ConfigDrawer />
        <ProfileDropdown />
      </Header>

      <Main className='flex flex-1 flex-col gap-4 sm:gap-6'>
        <div>
          <Button
            type='button'
            variant='ghost'
            size='sm'
            className='-ms-2 mb-2'
            onClick={goBack}
          >
            <ArrowLeft className='size-4' />
            {t('actions.backToList')}
          </Button>
          <h2 className='text-2xl font-bold tracking-tight'>
            {t('new.title')}
          </h2>
          <p className='text-muted-foreground'>{t('new.description')}</p>
        </div>

        <form
          onSubmit={(event) => void handleSubmit(event)}
          className='grid max-w-3xl gap-6'
          noValidate
        >
          <div className='grid gap-2'>
            <Label htmlFor='role-name'>{t('new.name')}</Label>
            <Input
              id='role-name'
              value={name}
              autoComplete='off'
              maxLength={ROLE_NAME_MAX_LENGTH}
              aria-invalid={Boolean(nameError)}
              onChange={(event) => setName(event.target.value)}
            />
            <p className='text-xs text-muted-foreground'>{t('new.nameHint')}</p>
            {nameError && (
              <p role='alert' className='text-sm text-destructive'>
                {nameError}
              </p>
            )}
          </div>

          <div className='grid gap-2'>
            <Label htmlFor='role-description'>
              {t('new.descriptionLabel')}
            </Label>
            <Textarea
              id='role-description'
              value={description}
              autoComplete='off'
              maxLength={ROLE_DESCRIPTION_MAX_LENGTH}
              aria-invalid={Boolean(descriptionError)}
              onChange={(event) => setDescription(event.target.value)}
            />
            {descriptionError && (
              <p role='alert' className='text-sm text-destructive'>
                {descriptionError}
              </p>
            )}
          </div>

          <div className='grid gap-3'>
            <h3 className='text-base font-semibold'>{t('new.permissions')}</h3>

            {permissionsQuery.isPending ? (
              <div className='grid gap-3'>
                <Skeleton className='h-28 w-full' />
                <Skeleton className='h-28 w-full' />
              </div>
            ) : permissionsQuery.isError ? (
              <div className='grid justify-items-start gap-2 rounded-md border p-4'>
                <div className='flex items-center gap-2 text-sm font-medium'>
                  <AlertTriangle className='size-4 text-destructive' />
                  {t('states.permissionsLoadError')}
                </div>
                <Button
                  type='button'
                  variant='outline'
                  size='sm'
                  onClick={() => void permissionsQuery.refetch()}
                >
                  {t('states.permissionsRetry')}
                </Button>
              </div>
            ) : (
              <PermissionMatrix
                groups={groups}
                value={selected}
                onChange={setSelected}
                readOnly={submitting}
              />
            )}

            {permissionError && (
              <p role='alert' className='text-sm text-destructive'>
                {permissionError}
              </p>
            )}
          </div>

          {formError && (
            <ApiErrorAlert error={submitError} message={formError} />
          )}

          <div className='flex items-center gap-2'>
            <Button type='submit' disabled={submitting}>
              {submitting ? t('new.submitting') : t('new.submit')}
            </Button>
            <Button
              type='button'
              variant='outline'
              disabled={submitting}
              onClick={goBack}
            >
              {tCommon('actions.cancel')}
            </Button>
          </div>
        </form>
      </Main>

      <UnsavedChangesDialog
        blocker={blocker}
        title={t('new.discardTitle')}
        description={t('new.discardDescription')}
        discardLabel={t('new.discard')}
        continueLabel={t('new.continueEditing')}
      />
    </>
  )
}
