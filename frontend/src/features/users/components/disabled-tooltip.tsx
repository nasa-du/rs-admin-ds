import type { ReactNode } from 'react'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'

type DisabledTooltipProps = {
  reason?: string
  side?: React.ComponentProps<typeof TooltipContent>['side']
  children: ReactNode
}

/**
 * Explains why a target-level action is disabled. Without a reason the child
 * renders untouched, so callers can use this unconditionally.
 */
export function DisabledTooltip({
  reason,
  side = 'left',
  children,
}: DisabledTooltipProps) {
  if (!reason) return <>{children}</>

  return (
    <TooltipProvider delayDuration={0}>
      <Tooltip>
        <TooltipTrigger asChild>
          <span className='block' tabIndex={0}>
            {children}
          </span>
        </TooltipTrigger>
        <TooltipContent side={side}>{reason}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  )
}
