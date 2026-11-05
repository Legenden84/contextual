import type { HTMLAttributes } from 'react'
import { cn } from '@/lib/utils'

export function Kbd({ className, ...rest }: HTMLAttributes<HTMLElement>) {
  return (
    <kbd
      className={cn(
        'ml-2 rounded-sm border border-border-strong px-1 font-mono text-[11px] leading-[14px] text-ink-muted',
        className,
      )}
      {...rest}
    />
  )
}
