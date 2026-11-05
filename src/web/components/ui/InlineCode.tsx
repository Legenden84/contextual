import type { HTMLAttributes } from 'react'
import { cn } from '@/lib/utils'

/** Inline code pill in chat prose: `IOrderRepository`. */
export function InlineCode({ className, ...rest }: HTMLAttributes<HTMLElement>) {
  return (
    <code
      className={cn(
        'rounded-sm bg-pill-bg px-[5px] py-px font-mono text-[12px] leading-[18px] whitespace-nowrap text-ink-strong',
        className,
      )}
      {...rest}
    />
  )
}
