/**
 * Load generator for the acceptance harness (spec §12.6).
 *
 * Runs inside the isolated `perf` Compose service on the edge network, so it
 * reaches the API and metrics listeners directly without publishing either
 * port. It provisions fixture users through the real API, then measures:
 *
 *   - login:   20 concurrent password logins (p95 ≤ 1 s)
 *   - API:     50 concurrent authenticated users listing users (p95 ≤ 300 ms)
 *   - refresh: 20 concurrent refreshes of one session (p95 ≤ 200 ms, grace
 *              must return one identical successor and no reuse detection)
 *
 * Progress goes to stderr; exactly one JSON result object goes to stdout for
 * scripts/perf.sh to render and archive. Run it through scripts/perf.sh.
 */

type JsonRecord = Record<string, unknown>

type CallResult = {
  status: number
  body: JsonRecord
  text: string
  cookie: string | null
  headers: Headers
  ms: number
}

type LoginResult = {
  ok: boolean
  status: number
  code: string
  ms: number
  token: string
  cookie: string | null
  mustChangePassword: boolean
}

const BASE = requiredEnv('PERF_BASE_URL').replace(/\/+$/, '')
const METRICS_URL = requiredEnv('PERF_METRICS_URL')
const ORIGIN = process.env.PERF_ORIGIN ?? 'https://admin.example.com'
const ADMIN_USERNAME = process.env.PERF_ADMIN_USERNAME ?? 'ops.perf'
const ADMIN_TEMP_PASSWORD = requiredEnv('PERF_ADMIN_TEMP_PASSWORD')
const ADMIN_PASSWORD = requiredEnv('PERF_ADMIN_PASSWORD')
const USER_PASSWORD = requiredEnv('PERF_USER_PASSWORD')
const USER_PASSWORD_FINAL =
  process.env.PERF_USER_PASSWORD_FINAL ?? `${USER_PASSWORD}x`

const USERS = envInt('PERF_USERS', 50)
const LOGIN_CONCURRENCY = envInt('PERF_LOGIN_CONCURRENCY', 20)
const LOGIN_ROUNDS = envInt('PERF_LOGIN_ROUNDS', 4)
const API_CONCURRENCY = envInt('PERF_API_CONCURRENCY', 50)
const API_ROUNDS = envInt('PERF_API_ROUNDS', 10)
const REFRESH_CONCURRENCY = envInt('PERF_REFRESH_CONCURRENCY', 20)
const REFRESH_ROUNDS = envInt('PERF_REFRESH_ROUNDS', 3)

const LOGIN_P95_LIMIT_MS = envInt('PERF_LOGIN_P95_MS', 1_000)
const API_P95_LIMIT_MS = envInt('PERF_API_P95_MS', 300)
const REFRESH_P95_LIMIT_MS = envInt('PERF_REFRESH_P95_MS', 200)

function requiredEnv(name: string): string {
  const value = process.env[name]
  if (!value) throw new Error(`${name} is required; run through scripts/perf.sh`)
  return value
}

function envInt(name: string, fallback: number): number {
  const raw = process.env[name]
  if (!raw) return fallback
  const value = Number.parseInt(raw, 10)
  if (!Number.isFinite(value) || value <= 0) {
    throw new Error(`${name} must be a positive integer`)
  }
  return value
}

function log(message: string): void {
  console.error(`perf: ${message}`)
}

/** One HTTP call with the browser headers the API expects. */
async function call(
  path: string,
  init: {
    method?: string
    token?: string
    cookie?: string
    body?: unknown
  } = {}
): Promise<CallResult> {
  const headers: Record<string, string> = {
    Origin: ORIGIN,
    // Harmless on Bearer endpoints, required by the CSRF guard on the
    // cookie-authenticated ones.
    'x-requested-with': 'XMLHttpRequest',
  }
  if (init.token) headers.Authorization = `Bearer ${init.token}`
  if (init.cookie) headers.Cookie = `__Host-refresh=${init.cookie}`
  if (init.body !== undefined) headers['Content-Type'] = 'application/json'

  const started = performance.now()
  const response = await fetch(`${BASE}${path}`, {
    method: init.method ?? 'GET',
    headers,
    body: init.body === undefined ? undefined : JSON.stringify(init.body),
  })
  const text = await response.text()
  const ms = performance.now() - started

  let body: JsonRecord = {}
  if (text) {
    try {
      body = JSON.parse(text) as JsonRecord
    } catch {
      body = {}
    }
  }

  return {
    status: response.status,
    body,
    text,
    cookie: refreshCookie(response.headers),
    headers: response.headers,
    ms,
  }
}

/** Extract the rotated `__Host-refresh` value from a response. */
function refreshCookie(headers: Headers): string | null {
  const raw = headers.get('set-cookie')
  if (!raw) return null
  const match = /(?:^|,\s*)__Host-refresh=([^;]*)/.exec(raw)
  return match ? match[1] : null
}

async function login(identifier: string, password: string): Promise<LoginResult> {
  const result = await call('/api/v1/auth/login', {
    method: 'POST',
    body: { identifier, password },
  })
  const user = (result.body.user ?? {}) as JsonRecord
  return {
    ok: result.status === 200 && typeof result.body.access_token === 'string',
    status: result.status,
    code: String(result.body.code ?? ''),
    ms: result.ms,
    token: String(result.body.access_token ?? ''),
    cookie: result.cookie,
    mustChangePassword: user.must_change_password === true,
  }
}

async function tryLogin(
  identifier: string,
  password: string
): Promise<LoginResult | null> {
  const result = await login(identifier, password)
  return result.ok ? result : null
}

/** Establish a full (non-restricted) admin session, tolerating a kept stack. */
async function ensureAdmin(): Promise<{
  token: string
  cookie: string | null
}> {
  const existing = await tryLogin(ADMIN_USERNAME, ADMIN_PASSWORD)
  if (existing) return { token: existing.token, cookie: existing.cookie }

  const temporary = await login(ADMIN_USERNAME, ADMIN_TEMP_PASSWORD)
  if (!temporary.ok) {
    throw new Error(
      `admin login failed (${temporary.status} ${temporary.code}); check PERF_ADMIN_*`
    )
  }
  if (!temporary.mustChangePassword) {
    return { token: temporary.token, cookie: temporary.cookie }
  }

  const changed = await call('/api/v1/auth/change-password', {
    method: 'POST',
    token: temporary.token,
    cookie: temporary.cookie ?? undefined,
    body: {
      current_password: ADMIN_TEMP_PASSWORD,
      new_password: ADMIN_PASSWORD,
    },
  })
  if (changed.status !== 200) {
    throw new Error(
      `admin password change failed (${changed.status} ${String(changed.body.code ?? '')})`
    )
  }
  return {
    token: String(changed.body.access_token ?? ''),
    cookie: changed.cookie ?? temporary.cookie,
  }
}

type AdminSession = {
  token: string
  reauth: Promise<void> | null
}

/** POST as the admin, reauthenticating once on `reauth_required`. */
async function adminPost(
  session: AdminSession,
  path: string,
  body: unknown
): Promise<CallResult> {
  const first = await call(path, {
    method: 'POST',
    token: session.token,
    body,
  })
  if (first.status !== 403 || first.body.code !== 'reauth_required') return first

  if (!session.reauth) {
    session.reauth = (async () => {
      const reauth = await call('/api/v1/auth/reauthenticate', {
        method: 'POST',
        token: session.token,
        body: { password: ADMIN_PASSWORD },
      })
      if (reauth.status !== 200) {
        throw new Error(
          `reauthentication failed (${reauth.status} ${String(reauth.body.code ?? '')})`
        )
      }
      session.token = String(reauth.body.access_token ?? '')
    })().finally(() => {
      session.reauth = null
    })
  }
  await session.reauth

  return call(path, { method: 'POST', token: session.token, body })
}

async function adminRoleId(session: AdminSession): Promise<string> {
  const result = await call('/api/v1/roles?page_size=200', {
    token: session.token,
  })
  if (result.status !== 200) {
    throw new Error(`listing roles failed (${result.status})`)
  }
  const items = (result.body.items ?? []) as JsonRecord[]
  const admin = items.find((role) => role.name === 'admin')
  if (!admin || typeof admin.id !== 'string') {
    throw new Error('the built-in admin role is missing')
  }
  return admin.id
}

type PerfUser = {
  username: string
  email: string
  token: string
  cookie: string | null
  password: string
}

/**
 * Create the fixture users through the real admin API, then complete each
 * user's forced password change so the session is unrestricted. The setup
 * cost is deliberately excluded from the percentile measurements.
 */
async function provisionUsers(session: AdminSession): Promise<PerfUser[]> {
  const adminRole = await adminRoleId(session)
  const users: PerfUser[] = []

  for (let index = 0; index < USERS; index += 1) {
    const suffix = String(index + 1).padStart(3, '0')
    users.push({
      username: `perf.user.${suffix}`,
      email: `perf.user.${suffix}@example.com`,
      token: '',
      cookie: null,
      password: USER_PASSWORD_FINAL,
    })
  }

  log(`provisioning ${users.length} fixture users through POST /users`)
  const batchSize = 4
  for (let offset = 0; offset < users.length; offset += batchSize) {
    const batch = users.slice(offset, offset + batchSize)
    await Promise.all(
      batch.map(async (user) => {
        const created = await adminPost(session, '/api/v1/users', {
          username: user.username,
          email: user.email,
          display_name: user.username,
          password: USER_PASSWORD,
          role_ids: [adminRole],
        })
        if (created.status !== 201) {
          throw new Error(
            `creating ${user.username} failed (${created.status} ${String(created.body.code ?? '')})`
          )
        }

        const first = await login(user.username, USER_PASSWORD)
        if (!first.ok) {
          throw new Error(
            `fixture login for ${user.username} failed (${first.status} ${first.code})`
          )
        }
        const changed = await call('/api/v1/auth/change-password', {
          method: 'POST',
          token: first.token,
          cookie: first.cookie ?? undefined,
          body: {
            current_password: USER_PASSWORD,
            new_password: USER_PASSWORD_FINAL,
          },
        })
        if (changed.status !== 200) {
          throw new Error(
            `fixture password change for ${user.username} failed (${changed.status} ${String(changed.body.code ?? '')})`
          )
        }
        user.token = String(changed.body.access_token ?? '')
        user.cookie = changed.cookie
      })
    )
    log(`provisioned ${Math.min(offset + batchSize, users.length)}/${users.length}`)
  }

  return users
}

type Summary = {
  samples: number
  p50_ms: number
  p95_ms: number
  max_ms: number
  mean_ms: number
  errors: number
}

function summarize(samples: number[], errors: number): Summary {
  if (samples.length === 0) {
    return { samples: 0, p50_ms: 0, p95_ms: 0, max_ms: 0, mean_ms: 0, errors }
  }
  const sorted = [...samples].sort((left, right) => left - right)
  const total = samples.reduce((sum, value) => sum + value, 0)
  return {
    samples: samples.length,
    p50_ms: round(percentile(sorted, 0.5)),
    p95_ms: round(percentile(sorted, 0.95)),
    max_ms: round(sorted[sorted.length - 1]),
    mean_ms: round(total / samples.length),
    errors,
  }
}

function percentile(sorted: number[], fraction: number): number {
  const rank = Math.ceil(fraction * sorted.length)
  return sorted[Math.min(Math.max(rank - 1, 0), sorted.length - 1)]
}

function round(value: number): number {
  return Math.round(value * 100) / 100
}

/** Sum a Prometheus counter, treating an absent series as zero. */
async function counterValue(name: string): Promise<number> {
  const response = await fetch(METRICS_URL)
  if (!response.ok) throw new Error(`metrics scrape failed (${response.status})`)
  const text = await response.text()
  let total = 0
  for (const line of text.split('\n')) {
    if (!line.startsWith(`${name} `) && !line.startsWith(`${name}{`)) continue
    const value = Number.parseFloat(line.slice(line.lastIndexOf(' ') + 1))
    if (Number.isFinite(value)) total += value
  }
  return total
}

type GaugeSet = {
  active: number | null
  idle: number | null
  max: number | null
}

async function poolGauges(): Promise<GaugeSet> {
  const response = await fetch(METRICS_URL)
  const text = await response.text()
  const read = (state: string): number | null => {
    const match = new RegExp(
      `^db_pool_connections\\{state="${state}"\\} ([0-9.]+)$`,
      'm'
    ).exec(text)
    return match ? Number.parseFloat(match[1]) : null
  }
  return { active: read('active'), idle: read('idle'), max: read('max') }
}

async function measureLogin(users: PerfUser[]): Promise<Summary> {
  const selected = users.slice(0, LOGIN_CONCURRENCY)
  const samples: number[] = []
  let errors = 0
  for (let roundIndex = 0; roundIndex < LOGIN_ROUNDS; roundIndex += 1) {
    const batch = await Promise.all(
      selected.map((user) => login(user.username, user.password))
    )
    for (const result of batch) {
      samples.push(result.ms)
      if (!result.ok) {
        errors += 1
        log(
          `login error status=${result.status} code=${result.code} ms=${round(result.ms)}`
        )
      }
    }
    log(`login round ${roundIndex + 1}/${LOGIN_ROUNDS} complete`)
  }
  return summarize(samples, errors)
}

async function measureApi(users: PerfUser[]): Promise<Summary> {
  const selected = users.slice(0, API_CONCURRENCY)
  const samples: number[] = []
  let errors = 0
  for (let roundIndex = 0; roundIndex < API_ROUNDS; roundIndex += 1) {
    const batch = await Promise.all(
      selected.map((user) =>
        call('/api/v1/users?page=1&page_size=20', { token: user.token })
      )
    )
    for (const result of batch) {
      samples.push(result.ms)
      if (result.status !== 200) {
        errors += 1
        log(`api error status=${result.status} code=${String(result.body.code ?? '')}`)
      }
    }
    log(`api round ${roundIndex + 1}/${API_ROUNDS} complete`)
  }
  return summarize(samples, errors)
}

type RefreshSummary = Summary & {
  cookie_mismatch_batches: number
  reuse_delta: number
}

async function measureRefresh(cookie: string): Promise<RefreshSummary> {
  const reuseBefore = await counterValue('auth_refresh_reuse_detected_total')
  const samples: number[] = []
  let errors = 0
  let cookieMismatchBatches = 0
  let current = cookie

  for (let roundIndex = 0; roundIndex < REFRESH_ROUNDS; roundIndex += 1) {
    const batch = await Promise.all(
      Array.from({ length: REFRESH_CONCURRENCY }, () =>
        call('/api/v1/auth/refresh', {
          method: 'POST',
          cookie: current,
          body: {},
        })
      )
    )
    for (const result of batch) {
      samples.push(result.ms)
      if (result.status !== 200) {
        errors += 1
        log(`refresh error status=${result.status} code=${String(result.body.code ?? '')}`)
      }
    }
    const successors = new Set(
      batch.map((result) => result.cookie).filter((value): value is string => Boolean(value))
    )
    if (successors.size !== 1) {
      cookieMismatchBatches += 1
      log(
        `refresh round ${roundIndex + 1}: ${successors.size} distinct successor cookies`
      )
    } else {
      const successor = [...successors][0]
      if (successor === current) {
        // Rotating the token is the point of this endpoint; an echo would
        // also keep the reuse counter at zero, so assert the change itself.
        cookieMismatchBatches += 1
        log(`refresh round ${roundIndex + 1}: successor equals the presented token`)
      } else {
        current = successor
      }
    }
    log(`refresh round ${roundIndex + 1}/${REFRESH_ROUNDS} complete`)
  }

  const reuseAfter = await counterValue('auth_refresh_reuse_detected_total')
  return {
    ...summarize(samples, errors),
    cookie_mismatch_batches: cookieMismatchBatches,
    reuse_delta: reuseAfter - reuseBefore,
  }
}

async function main(): Promise<void> {
  const started = performance.now()
  const admin = await ensureAdmin()
  const session: AdminSession = { token: admin.token, reauth: null }
  const users = await provisionUsers(session)
  const setupMs = performance.now() - started
  log(`setup complete in ${round(setupMs)} ms`)

  log(`measuring ${LOGIN_CONCURRENCY} concurrent logins × ${LOGIN_ROUNDS} rounds`)
  const loginSummary = await measureLogin(users)

  log(`measuring ${API_CONCURRENCY} concurrent API reads × ${API_ROUNDS} rounds`)
  const apiSummary = await measureApi(users)

  log(`measuring ${REFRESH_CONCURRENCY} concurrent refreshes × ${REFRESH_ROUNDS} rounds`)
  const refreshCookie = admin.cookie
  if (!refreshCookie) throw new Error('the admin session did not issue a refresh cookie')
  const refreshSummary = await measureRefresh(refreshCookie)

  const pool = await poolGauges()
  const ok =
    loginSummary.samples > 0 &&
    loginSummary.errors === 0 &&
    loginSummary.p95_ms <= LOGIN_P95_LIMIT_MS &&
    apiSummary.samples > 0 &&
    apiSummary.errors === 0 &&
    apiSummary.p95_ms <= API_P95_LIMIT_MS &&
    refreshSummary.samples > 0 &&
    refreshSummary.errors === 0 &&
    refreshSummary.p95_ms <= REFRESH_P95_LIMIT_MS &&
    refreshSummary.cookie_mismatch_batches === 0 &&
    refreshSummary.reuse_delta === 0

  const result = {
    ok,
    thresholds: {
      login_p95_ms: LOGIN_P95_LIMIT_MS,
      api_p95_ms: API_P95_LIMIT_MS,
      refresh_p95_ms: REFRESH_P95_LIMIT_MS,
    },
    concurrency: {
      login: LOGIN_CONCURRENCY,
      api: API_CONCURRENCY,
      refresh: REFRESH_CONCURRENCY,
    },
    setup: {
      users: users.length,
      duration_ms: round(setupMs),
    },
    login: loginSummary,
    api: apiSummary,
    refresh: refreshSummary,
    pool_last: pool,
    duration_ms: round(performance.now() - started),
  }

  console.log(JSON.stringify(result))
  if (!ok) process.exitCode = 1
}

await main()
