import { useTranslation } from 'react-i18next'
import { useHealthReady } from '@/api/generated/rs-admin'

// Static map: ADR-0007 forbids building translation keys at runtime.
const BACKEND_STATUS_KEYS = {
  checking: 'backendStatus.checking',
  ready: 'backendStatus.ready',
  unreachable: 'backendStatus.unreachable',
} as const

type BackendState = keyof typeof BACKEND_STATUS_KEYS

/**
 * Shows the result of the generated `/health/ready` client call. This is the
 * first consumer of the generated API client and proves the codegen pipeline
 * end to end. The sign-in page mounts it in development only because
 * `/health/*` is not exposed publicly in production (ADR-0008).
 */
export function BackendStatus() {
  const { t } = useTranslation()

  const { data, isError, isPending } = useHealthReady({
    query: {
      retry: false,
      refetchInterval: 30_000,
      staleTime: 10_000,
    },
  })

  const state: BackendState = isPending
    ? 'checking'
    : isError || data?.status !== 'ready'
      ? 'unreachable'
      : 'ready'

  return (
    <p role='status' className='mt-3 text-center text-xs text-muted-foreground'>
      {t(BACKEND_STATUS_KEYS[state])}
    </p>
  )
}
