import { cn } from '@/lib/utils'
import { Icon, type IconName } from './Icon'

export interface SegmentOption<T extends string> {
  value: T
  label: string
  icon?: IconName
}

export function Segmented<T extends string>({
  options,
  value,
  onChange,
  size = 'md',
  label,
  className,
}: {
  options: SegmentOption<T>[]
  value: T
  onChange: (value: T) => void
  size?: 'sm' | 'md'
  label?: string
  className?: string
}) {
  return (
    <span
      role="radiogroup"
      aria-label={label}
      className={cn('inline-flex gap-0.5 rounded-md border border-border bg-surface-300 p-0.5', className)}
    >
      {options.map((o) => {
        const active = o.value === value
        return (
          <button
            key={o.value}
            type="button"
            role="radio"
            aria-checked={active}
            onClick={() => onChange(o.value)}
            className={cn(
              'inline-flex cursor-pointer items-center gap-1 rounded-sm whitespace-nowrap text-ink-muted [&_svg]:size-3.5',
              size === 'md' ? 'h-5 px-2 text-ui-small' : 'h-[18px] px-1.5 text-[11px]',
              active && 'bg-surface-100 text-ink-strong shadow-[0_0_0_1px_var(--border-strong)]',
            )}
          >
            {o.icon && <Icon name={o.icon} />}
            {o.label}
          </button>
        )
      })}
    </span>
  )
}
