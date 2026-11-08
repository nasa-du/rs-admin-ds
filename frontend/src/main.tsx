import { StrictMode } from 'react'
import ReactDOM from 'react-dom/client'
import { QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider, createRouter } from '@tanstack/react-router'
import { queryClient } from '@/lib/query-client'
import { subscribeToAuthBroadcast } from '@/features/auth/auth-broadcast'
import {
  handlePasswordChangeRequired,
  onPasswordChangeRequired,
} from '@/features/auth/auth-interruptions'
import { authSession } from '@/features/auth/auth-session'
import { ReauthProvider } from '@/features/auth/reauth/reauth-dialog'
import { handleRemoteSignOut } from '@/features/auth/sign-out'
import { LocaleProvider } from './components/locale-provider'
import { DirectionProvider } from './context/direction-provider'
import { FontProvider } from './context/font-provider'
import { ThemeProvider } from './context/theme-provider'
// Initialize i18n before the first render
import './i18n'
// Generated Routes
import { routeTree } from './routeTree.gen'
// Styles
import './styles/index.css'

// Create a new router instance
const router = createRouter({
  routeTree,
  context: { queryClient },
  defaultPreload: 'intent',
  defaultPreloadStaleTime: 0,
})

// Register the router instance for type safety
declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}

function redirectToSignIn(redirect: string) {
  void router.navigate({
    to: '/sign-in',
    search: { redirect },
    replace: true,
  })
}

// A refresh failure clears the session inside `authSession`; move any page
// other than sign-in to sign-in with the current path as the redirect target
// (ADR-0006 §3).
authSession.onSessionInvalid(() => {
  const { pathname, href } = router.state.location
  if (pathname !== '/sign-in') redirectToSignIn(href)
})

// A restricted session that reaches a business endpoint is moved to the
// password-change page (spec §9.5).
onPasswordChangeRequired(() => {
  const { pathname, href } = router.state.location
  handlePasswordChangeRequired({
    pathname,
    href,
    redirectToChangePassword: (redirect) => {
      void router.navigate({
        to: '/change-password',
        search: { redirect },
        replace: true,
      })
    },
  })
})

// Cross-tab sign-out sync (spec §9.6): other tabs clear state and protected
// pages return to sign-in.
subscribeToAuthBroadcast(() => {
  const { pathname, href } = router.state.location
  handleRemoteSignOut({
    signOut: () => authSession.signOut(),
    pathname,
    href,
    redirectToSignIn,
  })
})

// Start cold-start session restoration without blocking public routes. Route
// guards await the same single-flight `bootstrap()` promise.
void authSession.bootstrap()

// Render the app
const rootElement = document.getElementById('root')!
if (!rootElement.innerHTML) {
  const root = ReactDOM.createRoot(rootElement)
  root.render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <ThemeProvider>
          <FontProvider>
            <DirectionProvider>
              <LocaleProvider>
                <ReauthProvider>
                  <RouterProvider router={router} />
                </ReauthProvider>
              </LocaleProvider>
            </DirectionProvider>
          </FontProvider>
        </ThemeProvider>
      </QueryClientProvider>
    </StrictMode>
  )
}
