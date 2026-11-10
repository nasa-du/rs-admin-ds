import { useMemo, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate, useParams } from '@tanstack/react-router'
import { type ErrorTranslator, translateApiError } from '@/i18n/errors'
import { AlertTriangle, ArrowLeft } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import type {
  PermissionGroup,
  RoleDetail as RoleDetailModel,
  UserSelfResponse,
} from '@/api/generated/model'
import {
  getGetRoleQueryKey,
  getListRolesQueryKey,
  useGetRole,
  useListPermissions,
  useSetRolePermissions,
  useUpdateRole,
} from '@/api/generated/rs-admin'
import { markApiErrorHandled, toApiError } from '@/lib/api-error'
import { hasPermission } from '@/lib/permissions'
import {
  isBaselineRole,
  isSuperAdminRole,
  roleDisplayName,
} from '@/lib/role-labels'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Skeleton } from '@/components/ui/skeleton'
import { Textarea } from '@/components/ui/textarea'
import { ApiErrorAlert } from '@/components/api-error-alert'
import { ConfigDrawer } from '@/components/config-drawer'
import { DisabledTooltip } from '@/components/disabled-tooltip'
import { Header } from '@/components/layout/header'
import { Main } from '@/components/layout/main'
import { ProfileDropdown } from '@/components/profile-dropdown'
import { Search } from '@/components/search'
import { ThemeSwitch } from '@/components/theme-switch'
import { runWithReauth } from '@/features/auth/reauth/reauth'
import { ReauthCancelledError } from '@/features/auth/reauth/reauth-controller'
import { useAuth } from '@/features/auth/use-auth'
import { PermissionMatrix } from './components/permission-matrix'
import { RoleDeleteDialog } from './components/role-delete-dialog'
import { allPermissionKeys, orderedPermissionKeys } from './permission-catalog'
import { mapRoleServerError } from './role-errors'
import {
  ROLE_DESCRIPTION_MAX_LENGTH,
  ROLE_NAME_MAX_LENGTH,
  buildRoleUpdate,
  makeRoleMetadataSchema,
  makeRolePermissionsSchema,
  permissionsDirty,
} from './role-form'
import { UnsavedChangesDialog } from './unsaved-changes'
import { useUnsavedChangesGuard } from './use-unsaved-changes'

/**
 * Role detail/edit page (spec §9.9). Built-in roles and roles outside the
 * operator's effective permissions are read-only; custom manageable roles
 * save metadata and permissions in one action (PATCH then PUT).
 */
export function RoleDetail() {
  const { t } = useTranslation('roles')
  const { t: tCommon } = useTranslation('common')
  const { t: tErrors } = useTranslation('errors')
  const { roleId } = useParams({ from: '/_authenticated/roles/$roleId' })
  const navigate = useNavigate()
  const currentUser = useAuth()

  const roleQuery = useGetRole(roleId)
  const permissionsQuery = useListPermissions()

  function goBack() {
    void navigate({ to: '/roles' })
  }

  const errorCode = toApiError(roleQuery.error)?.code

  return (
    <>
      <Header fixed>
        <Search className='me-auto' />
        <ThemeSwitch />
        <ConfigDrawer />
        <ProfileDropdown />
      </Header>

      <Main className='flex flex-1 flex-col gap-4 sm:gap-6'>
        <nav className='flex flex-wrap items-center gap-1.5 text-sm text-muted-foreground'>
          <Button
            type='button'
            variant='ghost'
            size='sm'
            className='-ms-2 h-7 px-2'
            onClick={goBack}
          >
            <ArrowLeft className='size-4' />
            {t('title')}
          </Button>
          <span aria-hidden='true'>/</span>
          <span className='font-medium text-foreground'>
            {roleQuery.data
              ? roleDisplayName(t, roleQuery.data)
              : t('detail.title')}
          </span>
        </nav>

        {roleQuery.isPending ? (
          <div className='grid max-w-3xl gap-4'>
            <Skeleton className='h-8 w-56' />
            <Skeleton className='h-4 w-80' />
            <Skeleton className='h-40 w-full' />
          </div>
        ) : roleQuery.isError ? (
          errorCode === 'not_found' ? (
            <div className='grid justify-items-start gap-2'>
              <h2 className='text-2xl font-bold tracking-tight'>
                {t('states.notFound.title')}
              </h2>
              <p className='text-muted-foreground'>
                {t('states.notFound.description')}
              </p>
            </div>
          ) : (
            <div className='grid justify-items-start gap-3 rounded-md border p-4'>
              <div className='flex items-center gap-2 text-sm font-medium'>
                <AlertTriangle className='size-4 text-destructive' />
                {t('states.loadError')}
              </div>
              <ApiErrorAlert
                error={roleQuery.error}
                message={translateApiError(
                  tErrors as ErrorTranslator,
                  roleQuery.error
                )}
              />
              <Button
                type='button'
                variant='outline'
                onClick={() => void roleQuery.refetch()}
              >
                {tCommon('error.retry')}
              </Button>
            </div>
          )
        ) : roleQuery.data ? (
          <RoleDetailContent
            key={roleQuery.data.id}
            role={roleQuery.data}
            groups={permissionsQuery.data?.groups ?? []}
            permissionsPending={permissionsQuery.isPending}
            permissionsError={permissionsQuery.isError}
            onRetryPermissions={() => void permissionsQuery.refetch()}
            currentUser={currentUser}
          />
        ) : null}
      </Main>
    </>
  )
}

type RoleDetailContentProps = {
  role: RoleDetailModel
  groups: PermissionGroup[]
  permissionsPending: boolean
  permissionsError: boolean
  onRetryPermissions: () => void
  currentUser: UserSelfResponse | null
}

function RoleDetailContent({
  role,
  groups,
  permissionsPending,
  permissionsError,
  onRetryPermissions,
  currentUser,
}: RoleDetailContentProps) {
  const { t } = useTranslation('roles')
  const { t: tErrors } = useTranslation('errors')
  const { t: tValidation } = useTranslation('validation')
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  const [name, setName] = useState(role.name)
  const [description, setDescription] = useState(role.description ?? '')
  const [selected, setSelected] = useState<Set<string>>(
    () => new Set(role.permission_keys)
  )
  const [baseline, setBaseline] = useState(() => ({
    name: role.name,
    description: role.description ?? null,
    permissionKeys: [...role.permission_keys],
  }))
  const [nameError, setNameError] = useState<string | null>(null)
  const [descriptionError, setDescriptionError] = useState<string | null>(null)
  const [permissionError, setPermissionError] = useState<string | null>(null)
  const [formError, setFormError] = useState<string | null>(null)
  const [submitError, setSubmitError] = useState<unknown>(null)
  const [partialFailure, setPartialFailure] = useState(false)
  const [saving, setSaving] = useState(false)
  const [deleteOpen, setDeleteOpen] = useState(false)

  const metadataSchema = useMemo(
    () => makeRoleMetadataSchema(tValidation),
    [tValidation]
  )
  const permissionsSchema = useMemo(
    () => makeRolePermissionsSchema(tValidation),
    [tValidation]
  )

  const isBuiltin = role.is_builtin
  const manageable = role.manageable
  const canUpdate = hasPermission(currentUser, 'roles.update')
  const canDelete = hasPermission(currentUser, 'roles.delete')
  const canReadUsers = hasPermission(currentUser, 'users.read')
  const editable = !isBuiltin && manageable && canUpdate

  const metadataPatch = editable
    ? buildRoleUpdate(baseline, { name, description })
    : null
  const permissionsAreDirty = editable
    ? permissionsDirty(baseline.permissionKeys, selected)
    : false
  const dirty = Boolean(metadataPatch) || permissionsAreDirty

  const { blocker, allowNextNavigation } = useUnsavedChangesGuard(dirty)

  const updateMutation = useUpdateRole({
    mutation: { onError: markApiErrorHandled },
  })
  const setPermissionsMutation = useSetRolePermissions({
    mutation: { onError: markApiErrorHandled },
  })

  const displayName = roleDisplayName(t, role)
  const isSuperAdmin = isSuperAdminRole(role)
  const isBaselineUser = isBaselineRole(role)

  const matrixValue: ReadonlySet<string> = isSuperAdmin
    ? new Set(allPermissionKeys(groups))
    : editable
      ? selected
      : new Set(role.permission_keys)

  const deleteReason = manageable ? undefined : t('detail.notManageableBanner')

  function validate(): boolean {
    let valid = true

    setNameError(null)
    setDescriptionError(null)
    setPermissionError(null)

    const metadata = metadataSchema.safeParse({ name, description })
    if (!metadata.success) {
      const issue = metadata.error.issues[0]
      if (issue?.path[0] === 'description') {
        setDescriptionError(issue.message)
      } else {
        setNameError(issue?.message ?? tValidation('invalid'))
      }
      valid = false
    }

    if (editable) {
      const permissions = permissionsSchema.safeParse({
        permissionKeys: [...selected],
      })
      if (!permissions.success) {
        setPermissionError(permissions.error.issues[0]?.message ?? null)
        valid = false
      }
    }

    return valid
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

  async function handleSave() {
    if (!editable || saving || !dirty) return

    setSubmitError(null)
    setFormError(null)
    setPartialFailure(false)
    if (!validate()) return

    const patch = buildRoleUpdate(baseline, { name, description })
    const savePermissions = permissionsDirty(baseline.permissionKeys, selected)
    let metadataSaved = false
    let savedRole: RoleDetailModel | undefined

    setSaving(true)

    try {
      if (patch) {
        savedRole = await runWithReauth(() =>
          updateMutation.mutateAsync({ id: role.id, data: patch })
        )
        metadataSaved = true
        setBaseline((previous) => ({
          name: patch.name ?? previous.name,
          description:
            patch.description !== undefined
              ? patch.description
              : previous.description,
          permissionKeys: previous.permissionKeys,
        }))
      }

      if (savePermissions) {
        const keys = orderedPermissionKeys(groups, selected)
        savedRole = await runWithReauth(() =>
          setPermissionsMutation.mutateAsync({
            id: role.id,
            data: { permission_keys: keys },
          })
        )
        setBaseline((previous) => ({
          ...previous,
          permissionKeys: keys,
        }))
      }

      if (savedRole) {
        queryClient.setQueryData(getGetRoleQueryKey(role.id), savedRole)
      }
      toast.success(t('detail.saved'))
      await queryClient.invalidateQueries({
        queryKey: getListRolesQueryKey(),
      })
    } catch (error) {
      if (error instanceof ReauthCancelledError) {
        if (metadataSaved && savePermissions) setPartialFailure(true)
        return
      }

      setSubmitError(error)
      mapServerError(error)

      if (metadataSaved && savePermissions) {
        setPartialFailure(true)
        void queryClient.invalidateQueries({
          queryKey: getGetRoleQueryKey(role.id),
        })
      }
    } finally {
      setSaving(false)
    }
  }

  function handleDeleted() {
    allowNextNavigation()
    void navigate({ to: '/roles' })
  }

  const saveButton = editable ? (
    <Button
      type='button'
      disabled={!dirty || saving}
      onClick={() => void handleSave()}
    >
      {saving ? t('detail.saving') : t('detail.save')}
    </Button>
  ) : null

  const deleteButton =
    isBuiltin || !canDelete ? null : (
      <DisabledTooltip reason={deleteReason}>
        <Button
          type='button'
          variant='destructive'
          disabled={!manageable}
          onClick={() => setDeleteOpen(true)}
        >
          {t('detail.delete')}
        </Button>
      </DisabledTooltip>
    )

  return (
    <>
      <div className='flex flex-wrap items-start justify-between gap-3'>
        <div className='grid gap-1'>
          <div className='flex flex-wrap items-center gap-2'>
            <h2 className='text-2xl font-bold tracking-tight'>{displayName}</h2>
            {isBuiltin && (
              <Badge variant='outline' className='font-mono text-[10px]'>
                {role.name}
              </Badge>
            )}
            {!isBuiltin && !manageable && (
              <Badge variant='secondary'>{t('type.readOnly')}</Badge>
            )}
          </div>
          {role.description && (
            <p className='text-muted-foreground'>{role.description}</p>
          )}
        </div>

        <div className='flex flex-wrap items-center gap-2'>
          {canReadUsers && (
            <Button
              type='button'
              variant='outline'
              onClick={() =>
                void navigate({
                  to: '/users',
                  search: { role_id: role.id },
                })
              }
            >
              {t('detail.viewUsers')}
            </Button>
          )}
          {saveButton}
          {deleteButton}
        </div>
      </div>

      {isSuperAdmin && (
        <div className='rounded-md border bg-muted/40 p-3 text-sm text-muted-foreground'>
          {t('detail.superAdminBanner')}
        </div>
      )}
      {isBuiltin && !isSuperAdmin && (
        <div className='rounded-md border bg-muted/40 p-3 text-sm text-muted-foreground'>
          {t('detail.builtinBanner')}
        </div>
      )}
      {!isBuiltin && !manageable && (
        <div className='rounded-md border bg-muted/40 p-3 text-sm text-muted-foreground'>
          {t('detail.notManageableBanner')}
        </div>
      )}

      {partialFailure && (
        <div
          role='alert'
          className='rounded-md border border-amber-500/50 bg-amber-500/10 p-3 text-sm'
        >
          {t('detail.partialFailure')}
        </div>
      )}

      <div className='grid max-w-3xl gap-6'>
        {editable ? (
          <div className='grid gap-4 sm:grid-cols-2'>
            <div className='grid gap-2'>
              <Label htmlFor='role-detail-name'>{t('detail.name')}</Label>
              <Input
                id='role-detail-name'
                value={name}
                autoComplete='off'
                maxLength={ROLE_NAME_MAX_LENGTH}
                aria-invalid={Boolean(nameError)}
                onChange={(event) => setName(event.target.value)}
              />
              {nameError && (
                <p role='alert' className='text-sm text-destructive'>
                  {nameError}
                </p>
              )}
            </div>
            <div className='grid gap-2'>
              <Label htmlFor='role-detail-description'>
                {t('detail.descriptionLabel')}
              </Label>
              <Textarea
                id='role-detail-description'
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
          </div>
        ) : (
          role.description && (
            <div className='grid gap-1'>
              <p className='text-sm font-medium'>
                {t('detail.descriptionLabel')}
              </p>
              <p className='text-sm text-muted-foreground'>
                {role.description}
              </p>
            </div>
          )
        )}

        <div className='grid gap-3'>
          <h3 className='text-base font-semibold'>{t('detail.permissions')}</h3>

          {permissionsPending ? (
            <div className='grid gap-3'>
              <Skeleton className='h-28 w-full' />
              <Skeleton className='h-28 w-full' />
            </div>
          ) : permissionsError ? (
            <div className='grid justify-items-start gap-2 rounded-md border p-4'>
              <div className='flex items-center gap-2 text-sm font-medium'>
                <AlertTriangle className='size-4 text-destructive' />
                {t('states.permissionsLoadError')}
              </div>
              <Button
                type='button'
                variant='outline'
                size='sm'
                onClick={onRetryPermissions}
              >
                {t('states.permissionsRetry')}
              </Button>
            </div>
          ) : isBaselineUser ? (
            <div className='grid gap-1 rounded-md border p-4'>
              <p className='font-medium'>{t('detail.userEmptyTitle')}</p>
              <p className='text-sm text-muted-foreground'>
                {t('detail.userEmptyDescription')}
              </p>
            </div>
          ) : (
            <PermissionMatrix
              groups={groups}
              value={matrixValue}
              onChange={setSelected}
              readOnly={!editable}
            />
          )}

          {permissionError && (
            <p role='alert' className='text-sm text-destructive'>
              {permissionError}
            </p>
          )}
        </div>

        {formError && <ApiErrorAlert error={submitError} message={formError} />}

        {editable && (
          <div className='flex items-center gap-2'>
            <Button
              type='button'
              disabled={!dirty || saving}
              onClick={() => void handleSave()}
            >
              {saving ? t('detail.saving') : t('detail.save')}
            </Button>
          </div>
        )}
      </div>

      <RoleDeleteDialog
        open={deleteOpen}
        role={role}
        currentUser={currentUser}
        onOpenChange={setDeleteOpen}
        onDeleted={handleDeleted}
      />

      <UnsavedChangesDialog
        blocker={blocker}
        title={t('detail.discardTitle')}
        description={t('detail.discardDescription')}
        discardLabel={t('detail.discard')}
        continueLabel={t('detail.continueEditing')}
      />
    </>
  )
}
