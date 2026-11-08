import { createFileRoute, redirect } from '@tanstack/react-router'
import { AuthenticatedLayout } from '@/components/layout/authenticated-layout'
import { authSession } from '@/features/auth/auth-session'
import { authStore } from '@/features/auth/auth-store'
import { decideProtectedRoute } from '@/features/auth/route-guards'
import { SessionBootstrapAwareError } from '@/features/errors/session-bootstrap-error'
import { SessionBootstrapUnavailableError } from '@/features/errors/session-bootstrap-unavailable-error'

export const Route = createFileRoute('/_authenticated')({
  beforeLoad: async ({ location }) => {
    const user = await authSession.bootstrap()
    const decision = decideProtectedRoute({
      user,
      authStatus: authStore.getState().authStatus,
      pathname: location.pathname,
      href: location.href,
    })

    switch (decision.kind) {
      case 'sign-in':
        throw redirect({
          to: '/sign-in',
          search: { redirect: decision.redirect },
          replace: true,
        })
      case 'change-password':
        throw redirect({
          to: '/change-password',
          search: { redirect: decision.redirect },
          replace: true,
        })
      case 'forbidden':
        throw redirect({
          to: '/403',
          search: { from: decision.from },
          replace: true,
        })
      case 'bootstrap-error':
        throw new SessionBootstrapUnavailableError()
    }
  },
  component: AuthenticatedLayout,
  errorComponent: SessionBootstrapAwareError,
})
