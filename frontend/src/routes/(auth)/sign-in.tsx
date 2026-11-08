import { z } from 'zod'
import { createFileRoute, redirect } from '@tanstack/react-router'
import { PAGE_TITLE_KEYS } from '@/i18n/page-titles'
import { authSession } from '@/features/auth/auth-session'
import { decideSignInRoute } from '@/features/auth/route-guards'
import { SignIn } from '@/features/auth/sign-in'

const searchSchema = z.object({
  redirect: z.string().optional(),
})

export const Route = createFileRoute('/(auth)/sign-in')({
  component: SignIn,
  validateSearch: searchSchema,
  staticData: { titleKey: PAGE_TITLE_KEYS.signIn },
  beforeLoad: async ({ search }) => {
    const user = await authSession.bootstrap()
    const decision = decideSignInRoute({ user, redirect: search.redirect })
    if (decision.kind === 'redirect') {
      throw redirect({ href: decision.href, replace: true })
    }
  },
})
