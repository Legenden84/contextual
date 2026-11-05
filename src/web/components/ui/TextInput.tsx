import type { InputHTMLAttributes, ReactNode } from 'react'
import { cn } from '@/lib/utils'
import { Icon, type IconName } from './Icon'

export interface TextInputProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'size'> {
  icon?: IconName
  size?: 'sm' | 'md'
  mono?: boolean
  /** Content after the input, e.g. a unit or a reveal button. */
  trailing?: ReactNode
  boxClassName?: string
}

export function TextInput({ icon, size = 'md', mono, trailing, className, boxClassName, ...rest }: TextInputProps) {
  return (
    <label
      className={cn(
        'inline-flex w-full items-center gap-1.5 rounded-md border border-border-strong bg-surface-100 px-2 text-ink focus-within:border-focus-ring',
        size === 'sm' ? 'h-6' : 'h-control',
        boxClassName,
      )}
    >
      {icon && <Icon name={icon} className="size-3.5 text-ink-muted" />}
      <input
        className={cn(
          'w-full min-w-0 bg-transparent text-ui-small text-ink outline-none placeholder:text-ink-muted',
          mono && 'font-mono',
          className,
        )}
        {...rest}
      />
      {trailing}
    </label>
  )
}
