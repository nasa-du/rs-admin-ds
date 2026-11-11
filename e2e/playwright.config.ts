import { defineConfig } from '@playwright/test'

const adminOrigin = (
  process.env.E2E_ADMIN_ORIGIN ?? 'https://admin.example.com'
).replace(/\/+$/, '')
const apiOrigin = (
  process.env.E2E_API_ORIGIN ?? 'https://api.example.com'
).replace(/\/+$/, '')

const adminHost = new URL(adminOrigin).hostname
const apiHost = new URL(apiOrigin).hostname

/**
 * Full-stack journeys against the Compose gateway (spec §12.5).
 *
 * The browser resolves the production-style host names to the local gateway
 * without touching `/etc/hosts`, so Origin, CORS, CSRF, Secure cookies and
 * SameSite all keep their real semantics. `ignoreHTTPSErrors` accepts the
 * self-signed development certificate; it does not relax cookie attributes
 * or CORS. Certificate and SNI wiring is covered by
 * `scripts/check-deploy-edge.sh`, not by these journeys.
 *
 * Run through `scripts/e2e.sh`: it resets an isolated Compose project, boots
 * a fresh database, bootstraps the super admin and passes the credentials.
 */
export default defineConfig({
  testDir: './tests',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 180_000,
  expect: { timeout: 15_000 },
  reporter: [
    ['list'],
    ['html', { open: 'never', outputFolder: 'playwright-report' }],
  ],
  outputDir: 'test-results',
  use: {
    baseURL: adminOrigin,
    ignoreHTTPSErrors: true,
    locale: 'en-US',
    timezoneId: 'UTC',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
    launchOptions: {
      args: [
        // Ignore any HTTP(S)_PROXY exported by the host/CI environment; the
        // stack is local and must be reached directly.
        '--proxy-server=direct://',
        '--proxy-bypass-list=*',
        `--host-resolver-rules=MAP ${adminHost} 127.0.0.1, MAP ${apiHost} 127.0.0.1`,
      ],
    },
  },
})
