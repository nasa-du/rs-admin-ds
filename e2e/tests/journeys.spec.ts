import { expect, test } from '@playwright/test'
import {
  ADMIN_ORIGIN,
  ADMIN_PASSWORD,
  ADMIN_TEMP_PASSWORD,
  ADMIN_USERNAME,
  API_ORIGIN,
  REFRESH_COOKIE,
  WRONG_PASSWORD,
  apiCall,
  apiCallWithReauth,
  assignRoleViaUi,
  browserRefresh,
  changePassword,
  completeReauthIfPrompted,
  createRoleViaUi,
  createUserViaUi,
  customRole,
  expectDashboard,
  expectRefreshRejected,
  loginThroughUi,
  newAppContext,
  openFirstAuditDetail,
  openUserDetail,
  openUsers,
  readRefreshCookie,
  selectAuditAction,
  setRefreshCookie,
  signInViaUi,
  signOutAllViaUi,
  signOutViaUi,
  submitSignIn,
  users,
  waitForApi,
} from './support'

/**
 * The nine key journeys from spec §12.5, run in order against one freshly
 * bootstrapped Compose stack. State is intentionally shared: journey 1 changes
 * the bootstrap password and later journeys use the users and roles created
 * earlier, just as an operator would.
 */
test.describe.configure({ mode: 'serial' })

/** Mirrors REFRESH_GRACE_SECONDS in backend/crates/api/src/auth.rs. */
const REFRESH_GRACE_SECONDS = 30

test('1. first deploy: bootstrap login, forced password change, Dashboard', async ({
  page,
  context,
}) => {
  await page.goto('/')
  await expect(page).toHaveURL(/\/sign-in/)

  const signIn = await signInViaUi(page, ADMIN_USERNAME, ADMIN_TEMP_PASSWORD)
  expect(signIn.response.status()).toBe(200)
  expect(signIn.body.user?.must_change_password).toBe(true)

  // The refresh cookie keeps every production attribute; the `__Host-`
  // prefix requires Secure, Path=/ and no Domain.
  const headers = await signIn.response.allHeaders()
  const setCookie = headers['set-cookie'] ?? ''
  expect(setCookie).toContain(`${REFRESH_COOKIE}=`)
  expect(setCookie).toContain('Path=/')
  expect(setCookie).toContain('HttpOnly')
  expect(setCookie).toContain('Secure')
  expect(setCookie).toContain('SameSite=Strict')
  expect(setCookie).toMatch(/Max-Age=43200/)
  expect(setCookie).not.toMatch(/Domain=/i)

  await expect(page).toHaveURL(/\/change-password/)
  await expect(page.getByText('You must change your password')).toBeVisible()

  // CORS and CSRF are live at the real edge: the SPA origin is allowed with
  // credentials, while a cookie-authenticated POST without the CSRF header is
  // refused. The probe does not rotate the refresh token. Reading the problem
  // body from the page already proves the browser accepted the CORS response.
  const probeResponse = page.waitForResponse(
    (response) =>
      response.request().method() === 'POST' &&
      new URL(response.url()).pathname === '/api/v1/auth/refresh'
  )
  const csrfProbe = page.evaluate(async ({ apiOrigin }) => {
    const response = await fetch(`${apiOrigin}/api/v1/auth/refresh`, {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json' },
      body: '{}',
    })
    return {
      status: response.status,
      body: (await response.json()) as { code?: string },
    }
  }, { apiOrigin: API_ORIGIN })

  const probeHeaders = (await probeResponse).headers()
  expect(probeHeaders['access-control-allow-origin']).toBe(ADMIN_ORIGIN)
  expect(probeHeaders['access-control-allow-credentials']).toBe('true')

  const probe = await csrfProbe
  expect(probe.status).toBe(403)
  expect(probe.body.code).toBe('csrf_failed')

  await changePassword(page, ADMIN_TEMP_PASSWORD, ADMIN_PASSWORD)
  await expectDashboard(page)

  const refreshCookie = (await context.cookies(API_ORIGIN)).find(
    (cookie) => cookie.name === REFRESH_COOKIE
  )
  expect(refreshCookie).toMatchObject({
    httpOnly: true,
    secure: true,
    sameSite: 'Strict',
    path: '/',
  })
  expect(refreshCookie?.expires).toBeGreaterThan(
    Math.floor(Date.now() / 1000) + 43_000
  )

  for (const name of ['Users', 'Roles', 'Audit logs']) {
    await expect(
      page.getByRole('link', { name, exact: true }).first()
    ).toBeVisible()
  }
})

test('2. admin creates a user; the user changes the temporary password and opens Users', async ({
  page,
  browser,
}) => {
  await loginThroughUi(page, ADMIN_USERNAME, ADMIN_PASSWORD)
  const created = await createUserViaUi(page, users.editor, 'Admin')

  const userContext = await newAppContext(browser)
  try {
    const userPage = await userContext.newPage()
    const login = await signInViaUi(
      userPage,
      users.editor.username,
      created.temporary_password
    )
    expect(login.response.status()).toBe(200)
    expect(login.body.user?.must_change_password).toBe(true)
    await expect(userPage).toHaveURL(/\/change-password/)

    await changePassword(
      userPage,
      created.temporary_password,
      users.editor.password
    )
    await expectDashboard(userPage)

    await openUsers(userPage)
    await expect(
      userPage.getByRole('row').filter({ hasText: users.editor.username })
    ).toBeVisible()
  } finally {
    await userContext.close()
  }
})

test('3. custom role and permissions gate the menu and routes after re-login', async ({
  page,
  browser,
}) => {
  await loginThroughUi(page, ADMIN_USERNAME, ADMIN_PASSWORD)
  await createRoleViaUi(page, customRole, ['View audit logs'])
  const created = await createUserViaUi(page, users.auditor)
  await assignRoleViaUi(page, users.auditor.username, customRole.name)

  const userContext = await newAppContext(browser)
  try {
    const userPage = await userContext.newPage()
    await signInViaUi(
      userPage,
      users.auditor.username,
      created.temporary_password
    )
    await expect(userPage).toHaveURL(/\/change-password/)
    await changePassword(
      userPage,
      created.temporary_password,
      users.auditor.password
    )
    await expectDashboard(userPage)

    await expect(
      userPage.getByRole('link', { name: 'Audit logs', exact: true }).first()
    ).toBeVisible()
    await expect(
      userPage.getByRole('link', { name: 'Users', exact: true })
    ).toHaveCount(0)
    await expect(
      userPage.getByRole('link', { name: 'Roles', exact: true })
    ).toHaveCount(0)

    await userPage.goto('/users')
    await expect(userPage).toHaveURL(/\/403\?from=/)
    await expect(
      userPage.getByText('Access Forbidden', { exact: true })
    ).toBeVisible()

    await userPage.goto('/audit-logs')
    await expect(
      userPage.getByRole('heading', { name: 'Audit logs', level: 2 })
    ).toBeVisible()
  } finally {
    await userContext.close()
  }
})

test('4. disable revokes the session; sign-in is 403; enable keeps the old session revoked', async ({
  page,
  browser,
}) => {
  const userContext = await newAppContext(browser)
  try {
    const userPage = await userContext.newPage()
    const oldToken = await loginThroughUi(
      userPage,
      users.editor.username,
      users.editor.password
    )
    const oldRefresh = await readRefreshCookie(userContext)

    await loginThroughUi(page, ADMIN_USERNAME, ADMIN_PASSWORD)
    const sheet = await openUserDetail(page, users.editor.username)
    await sheet.getByRole('button', { name: 'Disable user' }).click()

    const confirm = page.getByRole('alertdialog')
    const disable = waitForApi(
      page,
      'PUT',
      /^\/api\/v1\/users\/[0-9a-f-]{36}\/status$/
    )
    await confirm.getByRole('button', { name: 'Disable', exact: true }).click()
    await completeReauthIfPrompted(page, ADMIN_PASSWORD, disable)
    expect((await disable).status()).toBe(200)
    await expect(
      sheet.getByRole('button', { name: 'Enable user' })
    ).toBeVisible()

    const blocked = await apiCall(userPage, '/api/v1/auth/me', {
      token: oldToken,
    })
    expect(blocked.status).toBe(403)
    expect(blocked.json?.code).toBe('account_disabled')

    const refreshBlocked = await expectRefreshRejected(
      userPage,
      oldRefresh,
      [403]
    )
    expect(refreshBlocked.json?.code).toBe('account_disabled')

    await setRefreshCookie(userContext, undefined)
    const denied = await signInViaUi(
      userPage,
      users.editor.username,
      users.editor.password
    )
    expect(denied.response.status()).toBe(403)
    expect(denied.body.code).toBe('account_disabled')
    await expect(
      userPage.getByText('This account is disabled. Contact an administrator.')
    ).toBeVisible()

    const enable = waitForApi(
      page,
      'PUT',
      /^\/api\/v1\/users\/[0-9a-f-]{36}\/status$/
    )
    await sheet.getByRole('button', { name: 'Enable user' }).click()
    await completeReauthIfPrompted(page, ADMIN_PASSWORD, enable)
    expect((await enable).status()).toBe(200)
    await expect(
      sheet.getByRole('button', { name: 'Disable user' })
    ).toBeVisible()

    // Re-enabling restores login eligibility but never the old session.
    const stillRevoked = await apiCall(userPage, '/api/v1/auth/me', {
      token: oldToken,
    })
    expect(stillRevoked.status).toBe(401)
    await expectRefreshRejected(userPage, oldRefresh, [401])

    await loginThroughUi(
      userPage,
      users.editor.username,
      users.editor.password
    )
    await expectDashboard(userPage)
  } finally {
    await userContext.close()
  }
})

test('5. password reset revokes old sessions; temporary login forces a change', async ({
  page,
  browser,
}) => {
  const userContext = await newAppContext(browser)
  try {
    const userPage = await userContext.newPage()
    const oldToken = await loginThroughUi(
      userPage,
      users.auditor.username,
      users.auditor.password
    )
    const oldRefresh = await readRefreshCookie(userContext)

    await loginThroughUi(page, ADMIN_USERNAME, ADMIN_PASSWORD)
    const sheet = await openUserDetail(page, users.auditor.username)
    await sheet.getByRole('button', { name: 'Reset password' }).click()

    // The title changes to "Temporary password issued" after the request, so
    // use a stable locator rather than filtering on the initial title.
    const dialog = page.locator('[data-slot="dialog-content"]')
    await expect(
      dialog.getByRole('heading', { name: 'Reset password' })
    ).toBeVisible()

    const reset = waitForApi(
      page,
      'POST',
      /^\/api\/v1\/users\/[0-9a-f-]{36}\/reset-password$/
    )
    await dialog.getByRole('button', { name: 'Reset password' }).click()
    await completeReauthIfPrompted(page, ADMIN_PASSWORD, reset)
    const response = await reset
    expect(response.status()).toBe(200)
    const issued = (await response.json()) as { temporary_password: string }

    await expect(
      dialog.locator('#temporary-password-view')
    ).toHaveValue(issued.temporary_password)
    await dialog
      .getByRole('button', { name: 'I have saved this password' })
      .click()
    await expect(dialog).toBeHidden()

    expect(
      (
        await apiCall(userPage, '/api/v1/auth/me', { token: oldToken })
      ).status
    ).toBe(401)
    await expectRefreshRejected(userPage, oldRefresh, [401])

    await setRefreshCookie(userContext, undefined)
    const login = await signInViaUi(
      userPage,
      users.auditor.username,
      issued.temporary_password
    )
    expect(login.response.status()).toBe(200)
    expect(login.body.user?.must_change_password).toBe(true)
    await expect(userPage).toHaveURL(/\/change-password/)
    await expect(userPage.getByText('You must change your password')).toBeVisible()

    await changePassword(
      userPage,
      issued.temporary_password,
      users.auditor.changedPassword
    )
    await expectDashboard(userPage)

    expect(
      (
        await apiCall(userPage, '/api/v1/auth/me', { token: oldToken })
      ).status
    ).toBe(401)
    await expectRefreshRejected(userPage, oldRefresh, [401])
  } finally {
    await userContext.close()
  }
})

test('6. five failed logins lock the account; admin unlock restores sign-in', async ({
  page,
  browser,
}) => {
  const userContext = await newAppContext(browser)
  try {
    const userPage = await userContext.newPage()
    await userPage.goto('/sign-in')

    for (let attempt = 1; attempt <= 4; attempt += 1) {
      const failure = await submitSignIn(
        userPage,
        users.editor.username,
        WRONG_PASSWORD
      )
      expect(failure.response.status()).toBe(401)
      await expect(
        userPage.getByText('The username or password is incorrect.')
      ).toBeVisible()
    }

    const locked = await submitSignIn(
      userPage,
      users.editor.username,
      WRONG_PASSWORD
    )
    expect(locked.response.status()).toBe(429)
    expect(locked.body.code).toBe('login_locked')
    expect(Number(locked.response.headers()['retry-after'])).toBeGreaterThan(0)
    await expect(
      userPage.getByText(
        'Too many attempts. This account is temporarily locked. Try again later.'
      )
    ).toBeVisible()
    await expect(userPage.getByText(/Try again in \d+s\./)).toBeVisible()
    await expect(
      userPage.getByRole('button', { name: 'Sign in', exact: true })
    ).toBeDisabled()

    await loginThroughUi(page, ADMIN_USERNAME, ADMIN_PASSWORD)
    const sheet = await openUserDetail(page, users.editor.username)
    await expect(sheet.getByText('Locked', { exact: true }).first()).toBeVisible()

    const unlock = waitForApi(
      page,
      'POST',
      /^\/api\/v1\/users\/[0-9a-f-]{36}\/unlock$/
    )
    await sheet.getByRole('button', { name: 'Unlock user' }).click()
    expect((await unlock).status()).toBe(200)
    await expect(
      sheet.getByRole('button', { name: 'Unlock user' })
    ).toHaveCount(0)

    // Reloading clears the client-side Retry-After countdown; the server is
    // authoritative and the account is unlocked.
    await userPage.goto('/sign-in')
    await loginThroughUi(
      userPage,
      users.editor.username,
      users.editor.password
    )
    await expectDashboard(userPage)
  } finally {
    await userContext.close()
  }
})

test('7. audit page finds every event; opening a detail writes audit_log_accessed', async ({
  page,
}) => {
  await loginThroughUi(page, ADMIN_USERNAME, ADMIN_PASSWORD)
  await page.goto('/audit-logs')
  await expect(
    page.getByRole('heading', { name: 'Audit logs', level: 2 })
  ).toBeVisible()

  const actions = [
    'Bootstrap super admin',
    'User created',
    'User status changed',
    'User password reset',
    'Account locked',
    'User unlocked',
    'Password changed',
  ]

  for (const action of actions) {
    await selectAuditAction(page, action)
    await expect(
      page.getByRole('row').filter({ hasText: action }).first()
    ).toBeVisible()
  }

  await selectAuditAction(page, 'User created')
  await openFirstAuditDetail(page, 'User created')
  await openFirstAuditDetail(page, 'User created')

  // The page materialized its 30-day window when it loaded; the two detail
  // views happened after that boundary. Recalculate to include them.
  const recalculated = waitForApi(page, 'GET', '/api/v1/audit-logs')
  await page
    .getByRole('button', { name: 'Recalculate as Last 30 days' })
    .click()
  await recalculated

  await selectAuditAction(page, 'Audit details viewed')
  const accessed = page
    .getByRole('row')
    .filter({ hasText: 'Audit details viewed' })
  await expect(accessed.first()).toBeVisible()
  expect(await accessed.count()).toBeGreaterThanOrEqual(2)
})

test('8. cold start, multi-tab sign-out and sign-out everywhere', async ({
  browser,
}) => {
  const context = await newAppContext(browser)
  try {
    const page = await context.newPage()
    const firstToken = await loginThroughUi(
      page,
      users.editor.username,
      users.editor.password
    )

    const refresh = page.waitForResponse(
      (response) =>
        response.request().method() === 'POST' &&
        new URL(response.url()).pathname === '/api/v1/auth/refresh' &&
        response.status() === 200
    )
    await page.reload()
    const secondToken = (
      (await (await refresh).json()) as { access_token: string }
    ).access_token
    await expectDashboard(page)
    expect(secondToken).not.toBe(firstToken)

    const persisted = await page.evaluate(() =>
      JSON.stringify({ local: { ...localStorage }, session: { ...sessionStorage } })
    )
    expect(persisted).not.toContain(secondToken)

    // A second tab shares the BroadcastChannel: signing out in one tab sends
    // the other back to sign-in.
    const otherTab = await context.newPage()
    const otherRefresh = otherTab.waitForResponse(
      (response) =>
        response.request().method() === 'POST' &&
        new URL(response.url()).pathname === '/api/v1/auth/refresh' &&
        response.status() === 200
    )
    await otherTab.goto('/')
    const otherToken = (
      (await (await otherRefresh).json()) as { access_token: string }
    ).access_token
    await expectDashboard(otherTab)

    await signOutViaUi(page)
    await expect(otherTab).toHaveURL((url) => url.pathname === '/sign-in')
    expect(
      (await apiCall(otherTab, '/api/v1/auth/me', { token: otherToken })).status
    ).toBe(401)
    expect(await readRefreshCookie(context)).toBeUndefined()

    // Sign-out everywhere revokes sessions in independent browser contexts.
    const first = await newAppContext(browser)
    const second = await newAppContext(browser)
    try {
      const firstPage = await first.newPage()
      const secondPage = await second.newPage()
      await loginThroughUi(
        firstPage,
        users.editor.username,
        users.editor.password
      )
      const secondTokenAfterAll = await loginThroughUi(
        secondPage,
        users.editor.username,
        users.editor.password
      )

      await signOutAllViaUi(firstPage)
      await secondPage.goto('/')
      await expect(secondPage).toHaveURL((url) => url.pathname === '/sign-in')
      expect(
        (
          await apiCall(secondPage, '/api/v1/auth/me', {
            token: secondTokenAfterAll,
          })
        ).status
      ).toBe(401)
    } finally {
      await first.close()
      await second.close()
    }
  } finally {
    await context.close()
  }
})

test('9. old access tokens die after password change, revoke and refresh replay', async ({
  page,
  browser,
}) => {
  // --- Self-service password change invalidates every other session. ---
  const first = await newAppContext(browser)
  const second = await newAppContext(browser)
  try {
    const firstPage = await first.newPage()
    const secondPage = await second.newPage()

    const firstToken = await loginThroughUi(
      firstPage,
      users.editor.username,
      users.editor.password
    )
    const secondToken = await loginThroughUi(
      secondPage,
      users.editor.username,
      users.editor.password
    )
    const secondRefresh = await readRefreshCookie(second)

    await firstPage.goto('/settings/account')
    const rotatedToken = await changePassword(
      firstPage,
      users.editor.password,
      users.editor.changedPassword
    )
    expect(
      (await apiCall(firstPage, '/api/v1/auth/me', { token: rotatedToken }))
        .status
    ).toBe(200)
    await expect(firstPage.getByText('Password changed')).toBeVisible()

    expect(
      (await apiCall(firstPage, '/api/v1/auth/me', { token: firstToken })).status
    ).toBe(401)
    expect(
      (await apiCall(secondPage, '/api/v1/auth/me', { token: secondToken }))
        .status
    ).toBe(401)
    await expectRefreshRejected(secondPage, secondRefresh, [401])
    await secondPage.goto('/')
    await expect(secondPage).toHaveURL((url) => url.pathname === '/sign-in')
  } finally {
    await first.close()
    await second.close()
  }

  // --- Admin session revocation invalidates the target's tokens. ---
  const target = await newAppContext(browser)
  try {
    const targetPage = await target.newPage()
    const revokedToken = await loginThroughUi(
      targetPage,
      users.auditor.username,
      users.auditor.changedPassword
    )
    const revokedRefresh = await readRefreshCookie(target)

    const adminToken = await loginThroughUi(page, ADMIN_USERNAME, ADMIN_PASSWORD)
    const sheet = await openUserDetail(page, users.auditor.username)
    await sheet.getByRole('button', { name: 'Revoke all sessions' }).click()

    const confirm = page.getByRole('alertdialog')
    const revoke = waitForApi(
      page,
      'DELETE',
      /^\/api\/v1\/users\/[0-9a-f-]{36}\/sessions$/
    )
    await confirm.getByRole('button', { name: 'Revoke sessions' }).click()
    await completeReauthIfPrompted(page, ADMIN_PASSWORD, revoke)
    expect((await revoke).status()).toBe(204)

    expect(
      (
        await apiCall(targetPage, '/api/v1/auth/me', { token: revokedToken })
      ).status
    ).toBe(401)
    await expectRefreshRejected(targetPage, revokedRefresh, [401])
    await targetPage.goto('/')
    await expect(targetPage).toHaveURL((url) => url.pathname === '/sign-in')

    // --- A replay outside the grace window revokes the whole family. ---
    const created = await apiCallWithReauth(
      page,
      '/api/v1/users',
      {
        method: 'POST',
        token: adminToken,
        body: {
          username: users.replay.username,
          email: users.replay.email,
          display_name: users.replay.displayName,
        },
      },
      ADMIN_PASSWORD
    )
    expect(created.result.status).toBe(201)
    const temporaryPassword = (
      created.result.json as { temporary_password: string }
    ).temporary_password

    const replayContext = await newAppContext(browser)
    try {
      const replayPage = await replayContext.newPage()
      await signInViaUi(
        replayPage,
        users.replay.username,
        temporaryPassword
      )
      await expect(replayPage).toHaveURL(/\/change-password/)
      const accessToken = await changePassword(
        replayPage,
        temporaryPassword,
        users.replay.password
      )
      await expectDashboard(replayPage)

      const originalRefresh = await readRefreshCookie(replayContext)
      expect(originalRefresh).toBeTruthy()

      const rotation = await browserRefresh(replayPage)
      expect(rotation.status).toBe(200)
      const rotatedAccessToken = (rotation.json as { access_token: string })
        .access_token
      const rotatedRefresh = await readRefreshCookie(replayContext)
      expect(rotatedRefresh).toBeTruthy()
      expect(rotatedRefresh).not.toBe(originalRefresh)

      // Wait past the grace window so the replay is not idempotent.
      await replayPage.waitForTimeout((REFRESH_GRACE_SECONDS + 1) * 1_000)

      await setRefreshCookie(replayContext, originalRefresh)
      const replay = await browserRefresh(replayPage)
      expect(replay.status).toBe(401)
      expect(replay.json?.code).toBe('refresh_reuse_detected')
      expect(await readRefreshCookie(replayContext)).toBeUndefined()

      for (const token of [accessToken, rotatedAccessToken]) {
        expect(
          (await apiCall(replayPage, '/api/v1/auth/me', { token })).status
        ).toBe(401)
      }

      await setRefreshCookie(replayContext, rotatedRefresh)
      const successor = await browserRefresh(replayPage)
      expect(successor.status).toBe(401)

      const audit = await apiCall(
        page,
        '/api/v1/audit-logs?action=authn_refresh_reuse_detected',
        { token: created.token }
      )
      expect(audit.status).toBe(200)
      expect((audit.json as { total: number }).total).toBeGreaterThanOrEqual(1)
    } finally {
      await replayContext.close()
    }
  } finally {
    await target.close()
  }
})
