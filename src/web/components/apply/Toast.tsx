import { Button, IconButton } from '@/components/ui/Button'
import { Icon } from '@/components/ui/Icon'
import { useApp } from '@/state/app-context'

/** Confirmation after an apply. Stays until dismissed or the next apply. */
export function Toast() {
  const { toast, undoApply, dismissToast } = useApp()
  if (!toast) return null
  return (
    <div
      role="status"
      className="absolute right-4 bottom-10 z-11 flex w-[380px] items-center gap-3 rounded-lg border border-border-strong bg-surface-200 p-3 shadow-modal"
    >
      <span className="inline-flex size-6 flex-none items-center justify-center rounded-full bg-success text-surface-100">
        <Icon name="check" className="size-3.5" strokeWidth={2} />
      </span>
      <div>
        <b className="font-semibold text-ink-strong">{toast.title}</b>
        <div className="text-ui-small text-ink-muted">{toast.detail}</div>
      </div>
      <span className="flex-1" />
      {toast.undoable && (
        <Button size="xs" icon="undo" onClick={undoApply}>
          Undo
        </Button>
      )}
      <IconButton icon="close" label="Dismiss" onClick={dismissToast} />
    </div>
  )
}
