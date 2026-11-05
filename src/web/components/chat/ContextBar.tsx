import { Icon } from '@/components/ui/Icon'
import { EST_OUTPUT_TOKENS } from '@/data/defaults'
import { fmtK, fmtN } from '@/lib/format'
import { cn } from '@/lib/utils'
import { useApp } from '@/state/app-context'

const LEVEL_FILL = { ok: 'bg-accent', warn: 'bg-warning', danger: 'bg-danger' } as const
const LEVEL_INK = { ok: '', warn: 'text-warning', danger: 'text-danger' } as const

/** Context budget: files, rules, tokens against the deployment limit, and the cost per message. */
export function ContextBar() {
  const { totals, settings, deployment } = useApp()
  const dep = settings.deployments[deployment] ?? settings.deployments[0]
  const pct = Math.round(totals.share * 100)

  return (
    <div className="flex h-7 items-center gap-2 px-1 text-ui-small text-ink-muted">
      <span className="inline-flex items-center gap-1.5 whitespace-nowrap">
        <Icon name="files" className="size-3.5" />
        <b className="font-medium text-ink">{totals.files} files</b> · {totals.rules} rules · ~{fmtN(totals.tokens)} tokens
        {totals.conversationTokens > 0 && (
          <span title="The model re-reads the earlier conversation each turn (often from Azure's prompt cache); only new or changed files, pictures and your message are added.">
            {' '}
            (~{fmtN(totals.newTokens)} new)
          </span>
        )}
      </span>
      <span
        role="meter"
        aria-label="Context used"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pct}
        className="h-1 w-[140px] flex-none overflow-hidden rounded-full bg-surface-300"
      >
        <i
          className={cn('block h-full rounded-full', LEVEL_FILL[totals.level])}
          style={{ width: `${Math.min(100, totals.share * 100).toFixed(1)}%` }}
        />
      </span>
      <span className={cn('font-mono text-[11px] whitespace-nowrap', LEVEL_INK[totals.level])}>
        {pct}% of {fmtK(dep.limit)}
      </span>
      {totals.level !== 'ok' && (
        <>
          <span className={cn('inline-flex items-center gap-1 font-semibold whitespace-nowrap', LEVEL_INK[totals.level])}>
            <Icon name="warning" className="size-3.5" />
            {totals.level === 'warn' ? 'Near limit' : 'Over limit'}
          </span>
          {totals.hint && <span className="min-w-0 truncate">{totals.hint}</span>}
        </>
      )}
      <span className="flex-1" />
      {dep.inPrice > 0 || dep.outPrice > 0 ? (
        <span
          className="font-mono text-[11px] whitespace-nowrap"
          title={`Input at $${dep.inPrice.toFixed(2)}/1M + ~${fmtK(EST_OUTPUT_TOKENS)} output at $${dep.outPrice.toFixed(2)}/1M`}
        >
          ~${totals.cost.toFixed(3)} / message
        </span>
      ) : (
        <span className="text-[11px] whitespace-nowrap" title="Set AZURE_OPENAI_INPUT_PRICE_PER_1M and AZURE_OPENAI_OUTPUT_PRICE_PER_1M in .env.local">
          Cost: prices not set
        </span>
      )}
    </div>
  )
}
