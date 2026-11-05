import { cn } from '@/lib/utils'

export function Switch({
  on,
  onChange,
  label,
  className,
}: {
  on: boolean
  onChange?: () => void
  label?: string
  className?: string
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      onClick={(e) => {
        e.stopPropagation()
        onChange?.()
      }}
      className={cn(
        'relative inline-block h-3.5 w-[26px] flex-none cursor-pointer rounded-full border border-border-strong bg-surface-300',
        on && 'border-accent bg-accent',
        className,
      )}
    >
      <i
        className={cn(
          'absolute top-[2px] left-[2px] size-2 rounded-full bg-ink-muted transition-[left] duration-100',
          on && 'left-[14px] bg-on-accent',
        )}
      />
    </button>
  )
}

export function Radio({ on, onChange, label }: { on: boolean; onChange?: () => void; label?: string }) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={on}
      aria-label={label}
      onClick={onChange}
      className={cn(
        'inline-block size-3.5 cursor-pointer rounded-full border border-border-strong align-[-2px]',
        on && 'border-4 border-accent',
      )}
    />
  )
}
