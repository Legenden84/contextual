import type { ButtonHTMLAttributes, ReactNode } from 'react'
import { cn } from '@/lib/utils'
import { Icon, type IconName } from './Icon'
import { Kbd } from './Kbd'

type Variant = 'secondary' | 'primary' | 'ghost'
type Size = 'xs' | 'md' | 'lg' | 'icon'

const variants: Record<Variant, string> = {
  secondary: 'border-border-strong bg-surface-300 text-ink hover:bg-surface-hover',
  primary: 'border-accent bg-accent font-medium text-on-accent hover:bg-accent-hover',
  ghost: 'border-transparent bg-transparent text-ink-muted hover:bg-surface-hover hover:text-ink',
}

const sizes: Record<Size, string> = {
  xs: 'h-row gap-1 px-1.5 text-ui-small [&_svg]:size-3.5',
  md: 'h-control gap-1.5 px-2.5 text-ui-body',
  lg: 'h-9 gap-1.5 px-4 text-ui-prose',
  icon: 'size-control px-0',
}

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant
  size?: Size
  icon?: IconName
  kbd?: string
  children?: ReactNode
}

export function Button({ variant = 'secondary', size = 'md', icon, kbd, className, children, ...rest }: ButtonProps) {
  return (
    <button
      type="button"
      className={cn(
        'inline-flex cursor-pointer items-center justify-center rounded-md border whitespace-nowrap disabled:cursor-not-allowed disabled:opacity-50',
        variants[variant],
        sizes[size],
        className,
      )}
      {...rest}
    >
      {icon && <Icon name={icon} />}
      {children}
      {kbd && <Kbd className={variant === 'primary' ? 'border-on-accent text-on-accent' : undefined}>{kbd}</Kbd>}
    </button>
  )
}

export interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  icon: IconName
  label: string
  active?: boolean
}

/** 22px square icon-only button used in pane headers and toolbars. */
export function IconButton({ icon, label, active, className, ...rest }: IconButtonProps) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      aria-pressed={active}
      className={cn(
        'inline-flex size-row flex-none cursor-pointer items-center justify-center rounded-sm text-ink-muted hover:bg-surface-hover hover:text-ink',
        active && 'bg-surface-300 text-accent-ink',
        className,
      )}
      {...rest}
    >
      <Icon name={icon} />
    </button>
  )
}

/** Text button in the accent ink colour. */
export function LinkButton({ className, ...rest }: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      className={cn(
        'inline-flex cursor-pointer items-center gap-1 text-ui-small text-accent-ink hover:underline [&_svg]:size-3.5',
        className,
      )}
      {...rest}
    />
  )
}
