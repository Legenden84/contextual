import type { ReactNode } from 'react'
import type { CheckState } from '@/lib/tree'
import { cn } from '@/lib/utils'
import { Icon } from './Icon'

export interface CheckboxProps {
  state: CheckState | boolean
  onChange?: () => void
  disabled?: boolean
  label?: string
  className?: string
}

/** 14px tri-state checkbox. */
export function Checkbox({ state, onChange, disabled, label, className }: CheckboxProps) {
  const s: CheckState = state === true ? 'on' : state === false ? 'off' : state
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={s === 'on' ? true : s === 'mixed' ? 'mixed' : false}
      aria-label={label}
      disabled={disabled}
      onClick={(e) => {
        e.stopPropagation()
        onChange?.()
      }}
      className={cn(
        'inline-flex size-3.5 flex-none cursor-pointer items-center justify-center rounded-sm border border-border-strong bg-surface-100 text-on-accent',
        s !== 'off' && 'border-accent bg-accent',
        disabled && 'cursor-not-allowed opacity-45',
        className,
      )}
    >
      {s === 'on' && <Icon name="check" className="size-3" strokeWidth={2} />}
      {s === 'mixed' && <Icon name="minus" className="size-3" strokeWidth={2} />}
    </button>
  )
}

/** A checkbox (or switch) with its label; clicking the label toggles it. */
export function CheckLabel({
  control,
  onToggle,
  children,
  className,
}: {
  control: ReactNode
  onToggle?: () => void
  children: ReactNode
  className?: string
}) {
  return (
    <div
      className={cn('inline-flex cursor-pointer items-center gap-2 whitespace-nowrap select-none', className)}
      onClick={onToggle}
    >
      {control}
      {children}
    </div>
  )
}
