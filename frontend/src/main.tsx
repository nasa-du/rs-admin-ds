import { StrictMode } from 'react'
import ReactDOM from 'react-dom/client'
import { QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider, createRouter } from '@tanstack/react-router'
import { queryClient } from '@/lib/query-client'
import { authSession } from '@/features/auth/auth-session'
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
                <RouterProvider router={router} />
              </LocaleProvider>
            </DirectionProvider>
          </FontProvider>
        </ThemeProvider>
      </QueryClientProvider>
    </StrictMode>
  )
}
