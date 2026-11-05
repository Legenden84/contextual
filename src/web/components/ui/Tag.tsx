import type { HTMLAttributes } from 'react'
import { cn } from '@/lib/utils'

type Tone = 'default' | 'warn' | 'danger' | 'add' | 'skill' | 'excluded'

const tones: Record<Tone, string> = {
  default: 'bg-surface-300 text-ink-muted',
  warn: 'bg-warning-bg text-warning',
  danger: 'bg-surface-300 text-danger',
  add: 'bg-surface-300 text-success',
  skill: 'bg-surface-300 text-accent-ink',
  excluded: 'h-[15px] border border-border-strong bg-transparent text-[10px] text-ink-faint',
}

/** Small 16px label: "new file", "read-only", "Rule", "excluded"… */
export function Tag({ tone = 'default', className, ...rest }: { tone?: Tone } & HTMLAttributes<HTMLSpanElement>) {
  return (
    <span
      className={cn(
        'inline-flex h-4 flex-none items-center gap-[3px] rounded-sm px-[5px] text-[11px] leading-4 font-medium whitespace-nowrap [&_svg]:size-3',
        tones[tone],
        className,
      )}
      {...rest}
    />
  )
}

/** Round count badge, e.g. next to "Skills & Rules". */
export function Count({ className, ...rest }: HTMLAttributes<HTMLSpanElement>) {
  return (
    <span
      className={cn(
        'ml-1 inline-flex h-4 min-w-4 justify-center rounded-full bg-surface-300 px-1 text-[11px] leading-4 font-medium text-ink-muted',
        className,
      )}
      {...rest}
    />
  )
}

/** Outlined pill, e.g. the context chip on a user message. */
export function Chip({ className, ...rest }: HTMLAttributes<HTMLSpanElement>) {
  return (
    <span
      className={cn(
        'inline-flex h-5 items-center gap-1 rounded-full border border-border px-1.5 text-[11px] text-ink-muted [&_svg]:size-3',
        className,
      )}
      {...rest}
    />
  )
}
