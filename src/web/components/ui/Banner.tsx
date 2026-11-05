import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'
import { Icon } from './Icon'

/** Amber warning strip: conflicts, changed-on-disk notes. */
export function WarningBanner({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      role="status"
      className={cn(
        'flex items-center gap-2 border border-warning-border bg-warning-bg px-3 py-1.5 text-ui-small text-ink',
        className,
      )}
    >
      <Icon name="warning" className="text-warning" />
      {children}
    </div>
  )
}
