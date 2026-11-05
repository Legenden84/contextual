import type { ChangeStatus } from '@/data/types'
import { cn } from '@/lib/utils'

const LETTER_TONE: Record<ChangeStatus, string> = {
  A: 'text-success',
  M: 'text-modified',
  D: 'text-danger',
}
const LETTER_TITLE: Record<ChangeStatus, string> = { A: 'Added', M: 'Modified', D: 'Deleted' }

/** Boxed A / M / D letter. */
export function StatusLetter({ status, className }: { status: ChangeStatus; className?: string }) {
  return (
    <span
      title={LETTER_TITLE[status]}
      className={cn(
        'inline-flex size-3.5 flex-none items-center justify-center rounded-sm border border-current font-mono text-[10px] leading-[14px] font-semibold',
        LETTER_TONE[status],
        className,
      )}
    >
      {status}
    </span>
  )
}

/** +12 −3 in status colours; zero counts are left out. */
export function LineCounts({ adds, dels, className }: { adds: number; dels: number; className?: string }) {
  return (
    <span className={cn('inline-flex flex-none gap-1.5 font-mono text-[11px] font-semibold', className)}>
      {adds > 0 && <span className="text-success">+{adds}</span>}
      {dels > 0 && <span className="text-danger">−{dels}</span>}
    </span>
  )
}

/** GitHub-style +/− counts with a five-block bar. */
export function StatBar({ adds, dels }: { adds: number; dels: number }) {
  const total = adds + dels || 1
  const g = Math.round((adds / total) * 5)
  const r = Math.min(5 - g, Math.round((dels / total) * 5))
  return (
    <span className="inline-flex items-center gap-1.5 font-mono text-[12px]">
      <b className="font-semibold text-success">+{adds}</b>
      <b className="font-semibold text-danger">−{dels}</b>
      <span className="inline-flex gap-px">
        {Array.from({ length: 5 }, (_, i) => (
          <i
            key={i}
            className={cn('size-2 rounded-[1px] bg-surface-300', i < g ? 'bg-success' : i < g + r && 'bg-danger')}
          />
        ))}
      </span>
    </span>
  )
}
