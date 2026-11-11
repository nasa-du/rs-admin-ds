import {
  expect,
  type Browser,
  type BrowserContext,
  type Locator,
  type Page,
  type Response,
} from '@playwright/test'

/**
 * Shared helpers for the full-stack journeys.
 *
 * Everything here runs in a real Chromium against the production Compose
 * gateway. Direct API assertions still go through the page's `fetch`, so the
 * browser enforces CORS and Origin exactly as it does for the SPA.
 */

function required(name: string): string {
  const value = process.env[name]
  if (!value) throw new Error(`${name} is required; run through scripts/e2e.sh`)
  return value
}

export const ADMIN_ORIGIN = (
  process.env.E2E_ADMIN_ORIGIN ?? 'https://admin.example.com'
).replace(/\/+$/, '')
export const API_ORIGIN = (
  process.env.E2E_API_ORIGIN ?? 'https://api.example.com'
).replace(/\/+$/, '')
const API_HOST = new URL(API_ORIGIN).hostname

export const REFRESH_COOKIE = '__Host-refresh'

export const ADMIN_USERNAME = process.env.E2E_BOOTSTRAP_USERNAME ?? 'ops.e2e'
export const ADMIN_TEMP_PASSWORD = required('E2E_BOOTSTRAP_PASSWORD')
export const ADMIN_PASSWORD =
  process.env.E2E_ADMIN_PASSWORD ?? 'E2E-Admin-Harbor-2026!'

/** Password used by the failed-login journey. Never a real password. */
export const WRONG_PASSWORD = 'E2E-Wrong-Guess-2026!'

export const users = {
  editor: {
    username: 'e2e.editor',
    email: 'e2e.editor@example.com',
    displayName: 'E2E Editor',
    password: 'E2E-Editor-Orbit-2026!',
    changedPassword: 'E2E-Editor-Orbit-2026-2!',
  },
  auditor: {
    username: 'e2e.auditor',
    email: 'e2e.auditor@example.com',
    displayName: 'E2E Auditor',
    password: 'E2E-Auditor-Lantern-2026!',
    changedPassword: 'E2E-Auditor-Lantern-2026-2!',
  },
  replay: {
    username: 'e2e.replay',
    email: 'e2e.replay@example.com',
    displayName: 'E2E Replay',
    password: 'E2E-Replay-Cipher-2026!',
  },
} as const

export const customRole = {
  name: 'e2e-auditors',
  description: 'Read-only audit access created by the full-stack E2E journeys.',
} as const

export type ApiResult = {
  status: number
  json: JsonValue | null
  text: string
}

type JsonValue = Record<string, unknown>

export type SignInResult = {
  response: Response
  body: LoginBody
}

type LoginBody = {
  access_token?: string
  token_type?: string
  expires_in?: number
  user?: { must_change_password?: boolean } & Record<string, unknown>
  /** Problem details code when the login was refused (401/403/429). */
  code?: string
}

export function newAppContext(browser: Browser): Promise<BrowserContext> {
  // These contexts intentionally sit outside the test fixtures (the journeys
  // need independent cookie jars). Record video manually; failure screenshots
  // are still captured by the runner, but the config's trace fixture only
  // covers the default context.
  return browser.newContext({
    baseURL: ADMIN_ORIGIN,
    ignoreHTTPSErrors: true,
    locale: 'en-US',
    timezoneId: 'UTC',
    recordVideo: { dir: 'test-results/videos' },
  })
}

function pathnameOf(response: Response): string {
  return new URL(response.url()).pathname
}

function isLoginResponse(response: Response): boolean {
  return (
    response.request().method() === 'POST' &&
    pathnameOf(response) === '/api/v1/auth/login'
  )
}

/** Resolve once a mutation succeeds, ignoring the 403 `reauth_required` retry. */
export function waitForApi(
  page: Page,
  method: string,
  path: string | RegExp,
  statuses: number[] = [200, 201, 204]
): Promise<Response> {
  return page.waitForResponse((response) => {
    const pathname = pathnameOf(response)
    const matchesPath =
      typeof path === 'string' ? pathname === path : path.test(pathname)
    return (
      response.request().method() === method &&
      matchesPath &&
      statuses.includes(response.status())
    )
  })
}

/**
 * Fill and submit the sign-in form without navigating first. Repeated calls
 * drive the account-lockout journey.
 */
export async function submitSignIn(
  page: Page,
  identifier: string,
  password: string
): Promise<SignInResult> {
  await page.getByLabel('Username or email').fill(identifier)
  await page.getByLabel('Password').fill(password)

  const pending = page.waitForResponse(isLoginResponse)
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  const response = await pending
  const body = (await response.json().catch(() => ({}))) as SignInResult['body']

  return { response, body }
}

/** Open the sign-in page and submit the form once. */
export async function signInViaUi(
  page: Page,
  identifier: string,
  password: string
): Promise<SignInResult> {
  await page.goto('/sign-in')
  await expect(
    page.getByRole('button', { name: 'Sign in', exact: true })
  ).toBeVisible()
  return submitSignIn(page, identifier, password)
}

/** Sign in and assert the server accepted the credentials. */
export async function loginThroughUi(
  page: Page,
  identifier: string,
  password: string
): Promise<string> {
  const { response, body } = await signInViaUi(page, identifier, password)
  expect(response.status()).toBe(200)
  expect(body.access_token).toBeTruthy()
  return body.access_token as string
}

export async function expectDashboard(page: Page): Promise<void> {
  await expect(page).toHaveURL((url) => url.pathname === '/')
  await expect(
    page.getByRole('heading', { name: 'Dashboard', level: 1 })
  ).toBeVisible()
}

/**
 * Complete the app-wide reauthentication prompt if `runWithReauth` raised it.
 *
 * The mutation either settles on its own (no step-up) or blocks on the
 * prompt, so race the pre-registered success response against the dialog
 * instead of sleeping for a fixed window. This cannot miss a slow prompt and
 * costs nothing when reauthentication is not required.
 */
export async function completeReauthIfPrompted(
  page: Page,
  password: string,
  pending: Promise<unknown>
): Promise<void> {
  const dialog = page
    .locator('[data-slot="dialog-content"]')
    .filter({ hasText: 'Confirm your password' })
  const dialogVisible = dialog
    .waitFor({ state: 'visible', timeout: 15_000 })
    .then(() => true)
    .catch(() => false)

  const outcome = await Promise.race([
    pending.then(
      () => 'done' as const,
      () => 'done' as const
    ),
    dialogVisible.then((visible) =>
      visible ? ('reauth' as const) : ('timeout' as const)
    ),
  ])
  if (outcome !== 'reauth') return

  await dialog.getByLabel('Current password').fill(password)
  await dialog.getByRole('button', { name: 'Confirm', exact: true }).click()
  await expect(dialog).toBeHidden()
}

/** Change the current password from the forced-change or Account settings form. */
export async function changePassword(
  page: Page,
  currentPassword: string,
  newPassword: string
): Promise<string> {
  await page.getByLabel('Current password').fill(currentPassword)
  await page.getByLabel('New password', { exact: true }).fill(newPassword)
  await page.getByLabel('Confirm new password').fill(newPassword)

  const pending = waitForApi(page, 'POST', '/api/v1/auth/change-password')
  await page.getByRole('button', { name: 'Change password' }).click()
  const response = await pending
  expect(response.status()).toBe(200)
  const body = (await response.json()) as { access_token: string }
  return body.access_token
}

export async function openUsers(page: Page): Promise<void> {
  await page.getByRole('link', { name: 'Users', exact: true }).first().click()
  await expect(page).toHaveURL((url) => url.pathname === '/users')
  await expect(
    page.getByRole('heading', { name: 'Users', level: 2 })
  ).toBeVisible()
}

/** Filter the Users list by the exact username and open its detail sheet. */
export async function openUserDetail(
  page: Page,
  username: string
): Promise<Locator> {
  await page.goto(`/users?q=${encodeURIComponent(username)}`)
  await expect(
    page.getByRole('heading', { name: 'Users', level: 2 })
  ).toBeVisible()

  const row = page.getByRole('row').filter({ hasText: username }).first()
  await expect(row).toBeVisible()
  // Click the username cell: the actions cell contains buttons that stop
  // propagation, and the row centre is not a stable target.
  await row.getByRole('cell').first().click()

  const sheet = page.locator('[data-slot="sheet-content"]')
  await expect(sheet.getByRole('heading', { name: 'User details' })).toBeVisible()
  return sheet
}

export async function createUserViaUi(
  page: Page,
  user: { username: string; email: string; displayName: string },
  roleLabel?: string
): Promise<{ temporary_password: string; user: { id: string } }> {
  await page.goto('/users')
  await expect(
    page.getByRole('heading', { name: 'Users', level: 2 })
  ).toBeVisible()
  await page.getByRole('button', { name: 'Create user', exact: true }).click()

  // The title changes to "User created" once the request succeeds, so keep a
  // stable locator instead of filtering on the initial title text.
  const dialog = page.locator('[data-slot="dialog-content"]')
  await expect(dialog.getByRole('heading', { name: 'Create user' })).toBeVisible()

  await dialog.getByLabel('Username').fill(user.username)
  await dialog.getByLabel('Email').fill(user.email)
  await dialog.getByLabel('Display name').fill(user.displayName)

  if (roleLabel) {
    await dialog
      .locator('label')
      .filter({ hasText: new RegExp(`^${roleLabel}`) })
      .first()
      .click()
  }

  const pending = waitForApi(page, 'POST', '/api/v1/users', [201])
  await dialog.getByRole('button', { name: 'Create user', exact: true }).click()
  await completeReauthIfPrompted(page, ADMIN_PASSWORD, pending)
  const response = await pending
  expect(response.status()).toBe(201)
  const body = (await response.json()) as {
    temporary_password: string
    user: { id: string }
  }

  await expect(dialog.locator('#temporary-password-view')).toHaveValue(
    body.temporary_password
  )
  await dialog.getByRole('button', { name: 'I have saved this password' }).click()
  await expect(dialog).toBeHidden()

  return body
}

export async function createRoleViaUi(
  page: Page,
  role: { name: string; description: string },
  permissionLabels: string[]
): Promise<{ id: string }> {
  await page.goto('/roles')
  await expect(
    page.getByRole('heading', { name: 'Roles', level: 2 })
  ).toBeVisible()
  await page.getByRole('button', { name: 'Create role', exact: true }).click()
  await expect(page).toHaveURL((url) => url.pathname === '/roles/new')

  await page.locator('#role-name').fill(role.name)
  await page.locator('#role-description').fill(role.description)

  for (const label of permissionLabels) {
    await page.getByRole('checkbox', { name: label, exact: true }).check()
  }

  const pending = waitForApi(page, 'POST', '/api/v1/roles', [201])
  await page.getByRole('button', { name: 'Create role', exact: true }).click()
  await completeReauthIfPrompted(page, ADMIN_PASSWORD, pending)
  const response = await pending
  expect(response.status()).toBe(201)
  const body = (await response.json()) as { id: string }

  await expect(page).toHaveURL((url) => url.pathname === `/roles/${body.id}`)
  return body
}

export async function assignRoleViaUi(
  page: Page,
  username: string,
  roleName: string
): Promise<void> {
  const sheet = await openUserDetail(page, username)
  await sheet.getByRole('button', { name: 'Assign roles' }).click()

  const dialog = page.locator('[data-slot="dialog-content"]')
  await expect(dialog.getByRole('heading', { name: 'Assign roles' })).toBeVisible()

  await dialog
    .locator('label')
    .filter({ hasText: new RegExp(`^${roleName}`) })
    .first()
    .click()

  const pending = waitForApi(
    page,
    'PUT',
    /^\/api\/v1\/users\/[0-9a-f-]{36}\/roles$/
  )
  await dialog.getByRole('button', { name: 'Save roles' }).click()
  await completeReauthIfPrompted(page, ADMIN_PASSWORD, pending)
  const response = await pending
  expect(response.status()).toBe(200)
  await expect(dialog).toBeHidden()
}

export async function signOutViaUi(page: Page): Promise<void> {
  await page.locator('header button').filter({ hasText: 'RA' }).first().click()
  await page.getByRole('menuitem', { name: 'Sign out' }).click()

  const dialog = page.getByRole('alertdialog')
  await dialog.getByRole('button', { name: 'Sign out', exact: true }).click()
  await expect(page).toHaveURL((url) => url.pathname === '/sign-in')
}

export async function signOutAllViaUi(page: Page): Promise<void> {
  await page.goto('/settings/account')
  await page.getByRole('button', { name: 'Sign out all devices' }).click()

  const dialog = page.getByRole('alertdialog')
  await dialog.getByRole('button', { name: 'Sign out all devices' }).click()
  await expect(page).toHaveURL((url) => url.pathname === '/sign-in')
}

export async function selectAuditAction(
  page: Page,
  label: string
): Promise<void> {
  const select = page.getByRole('combobox', { name: 'Event type' })
  await select.click()
  await page.getByRole('option', { name: label, exact: true }).click()
  // TanStack Query may serve a repeated filter from its cache, so asserting
  // the rendered result is more reliable than waiting for a list request.
  await expect(select).toContainText(label)
  await expect(
    page.getByRole('row').filter({ hasText: label }).first()
  ).toBeVisible()
}

export async function openFirstAuditDetail(
  page: Page,
  actionLabel: string
): Promise<void> {
  const row = page.getByRole('row').filter({ hasText: actionLabel }).first()
  await expect(row).toBeVisible()

  const pending = page.waitForResponse(
    (response) =>
      response.request().method() === 'GET' &&
      /^\/api\/v1\/audit-logs\/[0-9a-f-]{36}$/.test(pathnameOf(response)) &&
      response.status() === 200
  )
  // The actor/target cells contain copy buttons that stop propagation, so
  // open the detail from the time cell.
  await row.getByRole('cell').first().click()

  const sheet = page.locator('[data-slot="sheet-content"]')
  await expect(sheet.getByRole('heading', { name: 'Audit event' })).toBeVisible()
  await pending

  await page.keyboard.press('Escape')
  await expect(sheet).toBeHidden()
}

/**
 * Call the API from the page's origin so Chromium performs the real CORS
 * preflight. `credentials: 'omit'` keeps Bearer calls independent of cookies.
 */
export async function apiCall(
  page: Page,
  path: string,
  options: {
    method?: string
    token?: string
    body?: unknown
    credentials?: RequestCredentials
  } = {}
): Promise<ApiResult> {
  return page.evaluate(
    async ({ apiOrigin, path, method, token, body, credentials }) => {
      const headers: Record<string, string> = { Accept: 'application/json' }
      if (token) headers.Authorization = `Bearer ${token}`
      if (body !== undefined) headers['Content-Type'] = 'application/json'

      const response = await fetch(`${apiOrigin}${path}`, {
        method,
        credentials,
        headers,
        body: body === undefined ? undefined : JSON.stringify(body),
      })
      const text = await response.text()
      let json: Record<string, unknown> | null = null
      try {
        json = JSON.parse(text) as Record<string, unknown>
      } catch {
        json = null
      }
      return { status: response.status, json, text }
    },
    {
      apiOrigin: API_ORIGIN,
      path,
      method: options.method ?? 'GET',
      token: options.token,
      body: options.body,
      credentials: options.credentials ?? 'omit',
    }
  )
}

/**
 * Call an API endpoint as the super admin, satisfying a step-up challenge
 * with `POST /auth/reauthenticate` and retrying once.
 */
export async function apiCallWithReauth(
  page: Page,
  path: string,
  options: {
    method?: string
    token?: string
    body?: unknown
    credentials?: RequestCredentials
  },
  adminPassword: string
): Promise<{ result: ApiResult; token: string }> {
  let token = options.token
  if (!token) throw new Error('apiCallWithReauth needs a bearer token')

  let result = await apiCall(page, path, options)

  if (result.status === 403 && result.json?.code === 'reauth_required') {
    const reauth = await apiCall(page, '/api/v1/auth/reauthenticate', {
      method: 'POST',
      token,
      body: { password: adminPassword },
    })
    expect(reauth.status).toBe(200)
    token = (reauth.json as { access_token: string }).access_token
    result = await apiCall(page, path, { ...options, token })
  }

  return { result, token }
}

/** Rotate the refresh token using the cookie currently in the browser jar. */
export async function browserRefresh(page: Page): Promise<ApiResult> {
  return page.evaluate(async ({ apiOrigin }) => {
    const response = await fetch(`${apiOrigin}/api/v1/auth/refresh`, {
      method: 'POST',
      credentials: 'include',
      headers: {
        'Content-Type': 'application/json',
        'X-Requested-With': 'XMLHttpRequest',
      },
      body: '{}',
    })
    const text = await response.text()
    let json: Record<string, unknown> | null = null
    try {
      json = JSON.parse(text) as Record<string, unknown>
    } catch {
      json = null
    }
    return { status: response.status, json, text }
  }, { apiOrigin: API_ORIGIN })
}

export async function readRefreshCookie(
  context: BrowserContext
): Promise<string | undefined> {
  const cookies = await context.cookies(API_ORIGIN)
  return cookies.find((cookie) => cookie.name === REFRESH_COOKIE)?.value
}

/** Replace the refresh cookie with an exact value (`undefined` removes it). */
export async function setRefreshCookie(
  context: BrowserContext,
  value: string | undefined
): Promise<void> {
  await context.clearCookies({ name: REFRESH_COOKIE })
  if (value === undefined) return

  await context.addCookies([
    {
      name: REFRESH_COOKIE,
      value,
      domain: API_HOST,
      path: '/',
      secure: true,
      httpOnly: true,
      sameSite: 'Strict',
    },
  ])
}

export async function expectRefreshRejected(
  page: Page,
  cookie: string | undefined,
  statuses: number[] = [401]
): Promise<ApiResult> {
  // A missing cookie would make the request unauthenticated and the 401
  // vacuous; fail before asserting the server response.
  expect(cookie, 'the refresh cookie must have been issued').toBeTruthy()
  await setRefreshCookie(page.context(), cookie)

  const result = await browserRefresh(page)
  expect(statuses).toContain(result.status)
  return result
}
