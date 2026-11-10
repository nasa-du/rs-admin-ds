import { useState } from 'react'
import i18n from '@/i18n'
import { describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import type { PermissionGroup } from '@/api/generated/model'
import { PermissionMatrix } from './permission-matrix'

const CATALOG: PermissionGroup[] = [
  {
    key: 'users',
    display_name_zh: '用户',
    display_name_en: 'Users',
    permissions: [
      {
        key: 'users.read',
        display_name_zh: '查看用户',
        display_name_en: 'View users',
        grantable: true,
      },
      {
        key: 'users.update',
        display_name_zh: '编辑用户',
        display_name_en: 'Edit users',
        grantable: false,
      },
      {
        key: 'users.create',
        display_name_zh: '创建用户',
        display_name_en: 'Create users',
        grantable: true,
      },
    ],
  },
  {
    key: 'roles',
    display_name_zh: '角色',
    display_name_en: 'Roles',
    permissions: [
      {
        key: 'roles.read',
        display_name_zh: '查看角色',
        display_name_en: 'View roles',
        grantable: true,
      },
    ],
  },
]

function Harness({
  initial = [],
  readOnly = false,
  onChange,
}: {
  initial?: string[]
  readOnly?: boolean
  onChange?: (next: Set<string>) => void
}) {
  const [value, setValue] = useState<Set<string>>(new Set(initial))

  return (
    <PermissionMatrix
      groups={CATALOG}
      value={value}
      readOnly={readOnly}
      onChange={(next) => {
        setValue(next)
        onChange?.(next)
      }}
    />
  )
}

describe('PermissionMatrix', () => {
  it('renders every group and permission with the backend display name', async () => {
    const screen = await render(<Harness />)

    await expect
      .element(screen.getByText('Users', { exact: true }))
      .toBeInTheDocument()
    await expect
      .element(screen.getByText('Roles', { exact: true }))
      .toBeInTheDocument()
    await expect
      .element(screen.getByRole('checkbox', { name: 'View users' }))
      .toBeInTheDocument()
    await expect
      .element(screen.getByRole('checkbox', { name: 'Edit users' }))
      .toBeInTheDocument()
    await expect
      .element(screen.getByText('users.read', { exact: true }))
      .toBeInTheDocument()
  })

  it('disables non-grantable permissions and explains why', async () => {
    const screen = await render(<Harness />)

    const checkbox = screen.getByRole('checkbox', { name: 'Edit users' })
    await expect.element(checkbox).toBeDisabled()

    const reason = i18n.t('roles:matrix.notGrantable')
    await expect.element(screen.getByText(reason)).toBeInTheDocument()
  })

  it('selects every grantable permission in a group and clears them again', async () => {
    const onChange = vi.fn()
    const screen = await render(<Harness onChange={onChange} />)
    const groupCheckbox = screen.getByRole('checkbox', {
      name: 'Select all Users permissions',
    })

    await userEvent.click(groupCheckbox)
    expect([...onChange.mock.calls[0][0]].sort()).toEqual([
      'users.create',
      'users.read',
    ])

    await expect.element(groupCheckbox).toBeChecked()
    await userEvent.click(groupCheckbox)
    expect([...onChange.mock.calls[1][0]]).toEqual([])
  })

  it('shows the group count and an indeterminate state', async () => {
    const screen = await render(<Harness initial={['users.read']} />)

    const groupCheckbox = screen.getByRole('checkbox', {
      name: 'Select all Users permissions',
    })
    await expect
      .element(groupCheckbox)
      .toHaveAttribute('data-state', 'indeterminate')
    await expect.element(screen.getByText('1 selected')).toBeInTheDocument()
  })

  it('toggles individual grantable permissions only', async () => {
    const onChange = vi.fn()
    const screen = await render(<Harness onChange={onChange} />)

    await userEvent.click(screen.getByRole('checkbox', { name: 'View users' }))
    expect([...onChange.mock.calls[0][0]]).toEqual(['users.read'])
    await expect
      .element(screen.getByRole('checkbox', { name: 'Edit users' }))
      .toBeDisabled()
    expect(onChange).toHaveBeenCalledTimes(1)
  })

  it('renders a read-only matrix without group toggles', async () => {
    const screen = await render(
      <Harness initial={['users.read', 'users.update']} readOnly />
    )

    await expect
      .element(screen.getByRole('checkbox', { name: 'View users' }))
      .toBeDisabled()
    await expect
      .element(screen.getByRole('checkbox', { name: 'View users' }))
      .toBeChecked()
    await expect
      .element(
        screen.getByRole('checkbox', { name: 'Select all Users permissions' })
      )
      .not.toBeInTheDocument()
  })
})
