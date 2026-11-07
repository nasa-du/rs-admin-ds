import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from '@tanstack/react-router'
import i18n from '@/i18n'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { DocumentTitle } from './document-title'

function createTestRouter() {
  const rootRoute = createRootRoute({
    component: () => (
      <>
        <DocumentTitle />
        <Outlet />
      </>
    ),
  })

  const indexRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: '/',
    staticData: { titleKey: PAGE_TITLE_KEYS.dashboard },
    component: () => <div>dashboard</div>,
  })

  return createRouter({
    routeTree: rootRoute.addChildren([indexRoute]),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
}

describe('DocumentTitle', () => {
  afterEach(async () => {
    document.title = 'rs-admin'
    await i18n.changeLanguage('en-US')
  })

  it('uses the deepest route title and updates when the locale changes', async () => {
    await i18n.changeLanguage('zh-CN')

    const router = createTestRouter()
    await render(<RouterProvider router={router} />)
    await router.load()

    await vi.waitFor(() => expect(document.title).toBe('仪表盘 · rs-admin'))

    await i18n.changeLanguage('en-US')

    await vi.waitFor(() => expect(document.title).toBe('Dashboard · rs-admin'))
  })
})
