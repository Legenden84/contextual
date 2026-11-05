import { useEffect, useRef, useState } from 'react'
import { Icon } from '@/components/ui/Icon'
import { fmtK } from '@/lib/format'
import { cn } from '@/lib/utils'
import { useApp } from '@/state/app-context'

/** Deployment dropdown in the composer. */
export function ModelPicker() {
  const { settings, deployment, setDeployment } = useApp()
  const [open, setOpen] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  const dep = settings.deployments[deployment] ?? settings.deployments[0]

  useEffect(() => {
    if (!open) return
    const close = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false)
    }
    const esc = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false)
    document.addEventListener('mousedown', close)
    document.addEventListener('keydown', esc)
    return () => {
      document.removeEventListener('mousedown', close)
      document.removeEventListener('keydown', esc)
    }
  }, [open])

  return (
    <div ref={root} className="relative">
      <button
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className="inline-flex h-6 cursor-pointer items-center gap-1.5 rounded-md border border-border bg-surface-100 pr-1.5 pl-2 text-ui-small whitespace-nowrap text-ink [&_svg]:size-3.5 [&_svg]:text-ink-muted"
      >
        <Icon name="cloud" />
        {dep.name} (Foundry)
        <Icon name="chevronDown" />
      </button>
      {open && (
        <div
          role="listbox"
          className="absolute bottom-full left-0 z-20 mb-1 w-[300px] rounded-md border border-border-strong bg-surface-200 p-1 shadow-popover"
        >
          {settings.deployments.map((d, i) => (
            <button
              key={d.name}
              type="button"
              role="option"
              aria-selected={i === deployment}
              onClick={() => {
                setDeployment(i)
                setOpen(false)
              }}
              className={cn(
                'flex w-full cursor-pointer items-center gap-2 rounded-sm px-2 py-1.5 text-left text-ui-body hover:bg-surface-hover',
                i === deployment && 'bg-surface-selected hover:bg-surface-selected',
              )}
            >
              <Icon name={i === deployment ? 'check' : 'cloud'} className={cn('size-3.5', i === deployment ? 'text-accent-ink' : 'text-ink-muted')} />
              <span className="flex-1">{d.name} (Foundry)</span>
              <span className="font-mono text-[11px] text-ink-muted">
                {fmtK(d.limit)} · ${d.inPrice.toFixed(2)}/${d.outPrice.toFixed(2)}
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  )
}
