import { useEffect, useRef, useState, type ReactNode } from 'react'
import { EST_OUTPUT_TOKENS } from '@/data/defaults'
import { fmtK, fmtN } from '@/lib/format'
import { cn } from '@/lib/utils'
import { useApp } from '@/state/app-context'

function Row({ label, children, strong }: { label: string; children: ReactNode; strong?: boolean }) {
  return (
    <div className="flex items-baseline justify-between gap-4 py-0.5">
      <span className="text-ink-muted">{label}</span>
      <span className={cn('font-mono text-[12px] whitespace-nowrap', strong ? 'font-semibold text-ink-strong' : 'text-ink')}>{children}</span>
    </div>
  )
}

const usd = (n: number) => `$${n.toFixed(n >= 1 ? 2 : 3)}`

/**
 * The status bar's token count; clicking it opens a usage summary for the current chat
 * (tokens and cost so far, from Azure's reported counts) and the estimate for the next message.
 */
export function UsagePopover() {
  const { totals, settings, deployment, thread, foundry } = useApp()
  const [open, setOpen] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  const dep = settings.deployments[deployment] ?? settings.deployments[0]
  const priced = dep.inPrice > 0 || dep.outPrice > 0
  const inputCost = (totals.chatInputTokens / 1e6) * dep.inPrice
  const outputCost = (totals.chatOutputTokens / 1e6) * dep.outPrice

  useEffect(() => {
    if (!open) return
    const close = (e: MouseEvent) => !root.current?.contains(e.target as Node) && setOpen(false)
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
        aria-haspopup="dialog"
        aria-expanded={open}
        title="Show usage for this chat"
        onClick={() => setOpen((o) => !o)}
        className={cn(
          'inline-flex h-6 cursor-pointer items-center gap-1 px-1 hover:bg-surface-hover hover:text-ink',
          open && 'bg-surface-hover text-ink',
        )}
      >
        {totals.files} files · {totals.rules} rules · ~{fmtN(totals.tokens)} tokens
      </button>

      {open && (
        <div
          role="dialog"
          aria-label="Usage for this chat"
          className="absolute right-0 bottom-full z-30 mb-1 w-[320px] rounded-md border border-border-strong bg-surface-200 p-3 text-ui-body shadow-popover"
        >
          <div className="mb-1.5 flex items-baseline justify-between">
            <span className="text-ui-caption text-ink-muted uppercase">This chat</span>
            <span className="text-ui-small text-ink-muted">
              {totals.chatReplies} {totals.chatReplies === 1 ? 'reply' : 'replies'}
            </span>
          </div>
          <Row label="Input tokens">
            {fmtN(totals.chatInputTokens)}
            {totals.chatCachedTokens > 0 && <span className="text-ink-muted"> ({fmtK(totals.chatCachedTokens)} cached)</span>}
          </Row>
          <Row label="Output tokens">{fmtN(totals.chatOutputTokens)}</Row>
          {priced ? (
            <>
              <Row label="Input cost">{usd(inputCost)}</Row>
              <Row label="Output cost">{usd(outputCost)}</Row>
              <div className="my-1.5 border-t border-border" />
              <Row label="Estimated cost so far" strong>
                {usd(totals.chatCost)}
              </Row>
            </>
          ) : (
            <p className="mt-1 text-ui-small text-ink-muted">
              {foundry.state === 'ready'
                ? 'Set AZURE_OPENAI_INPUT_PRICE_PER_1M and AZURE_OPENAI_OUTPUT_PRICE_PER_1M in .env.local to see costs.'
                : 'Costs appear once Foundry is configured; prices are read from .env.local.'}
            </p>
          )}

          <div className="mt-3 mb-1.5 text-ui-caption text-ink-muted uppercase">Next message</div>
          <Row label="Input (estimated)">~{fmtN(totals.tokens)}</Row>
          {thread.transcript.length > 0 && <Row label="Of which new">~{fmtN(totals.newTokens)}</Row>}
          {priced && <Row label={`Cost with ~${fmtK(EST_OUTPUT_TOKENS)} output`}>~{usd(totals.cost)}</Row>}

          <p className="mt-3 border-t border-border pt-2 text-ui-small text-ink-muted">
            {priced
              ? `Prices: $${dep.inPrice.toFixed(2)} input / $${dep.outPrice.toFixed(2)} output per 1M tokens (${dep.name}). `
              : ''}
            Uses the token counts Azure reported for each reply; cached input is counted at the full input price.
          </p>
        </div>
      )}
    </div>
  )
}
