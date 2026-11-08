import { z } from 'zod'
import { createFileRoute, redirect } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { authSession } from '@/features/auth/auth-session'
import { authStore } from '@/features/auth/auth-store'
import { ChangePassword } from '@/features/auth/change-password'
import { decideChangePasswordRoute } from '@/features/auth/route-guards'
import { SessionBootstrapAwareError } from '@/features/errors/session-bootstrap-error'
import { SessionBootstrapUnavailableError } from '@/features/errors/session-bootstrap-unavailable-error'

const searchSchema = z.object({
  redirect: z.string().optional(),
})

export const Route = createFileRoute('/(auth)/change-password')({
  component: ChangePassword,
  validateSearch: searchSchema,
  staticData: { titleKey: PAGE_TITLE_KEYS.changePassword },
  errorComponent: SessionBootstrapAwareError,
  beforeLoad: async ({ location }) => {
    const user = await authSession.bootstrap()
    const decision = decideChangePasswordRoute({
      user,
      authStatus: authStore.getState().authStatus,
      pathname: location.pathname,
      href: location.href,
    })

    if (decision.kind === 'sign-in') {
      throw redirect({
        to: '/sign-in',
        search: { redirect: decision.redirect },
        replace: true,
      })
    }
    if (decision.kind === 'bootstrap-error') {
      throw new SessionBootstrapUnavailableError()
    }
  },
})
