import { useState, type ReactNode } from 'react'
import { CodeEditor } from '@/components/code/CodeEditor'
import { WarningBanner } from '@/components/ui/Banner'
import { Button, IconButton, LinkButton } from '@/components/ui/Button'
import { CheckLabel } from '@/components/ui/Checkbox'
import { Icon } from '@/components/ui/Icon'
import { InlineCode } from '@/components/ui/InlineCode'
import { Segmented } from '@/components/ui/Segmented'
import { Radio, Switch } from '@/components/ui/Switch'
import { TextInput } from '@/components/ui/TextInput'
import { defaultExclusions, outputPrompt } from '@/data/sample'
import { testConnection } from '@/lib/api'
import { estimateTokens } from '@/lib/format'
import { cn } from '@/lib/utils'
import { useApp, type DiffMode, type Settings, type Theme } from '@/state/app-context'

const SECTIONS = ['Azure AI Foundry', 'Exclusions', 'Output format', 'Appearance', 'Keyboard'] as const
const sectionId = (s: string) => 'settings-' + s.toLowerCase().replace(/\W+/g, '-')

function Section({ title, intro, children }: { title: (typeof SECTIONS)[number]; intro?: ReactNode; children: ReactNode }) {
  return (
    <section
      id={sectionId(title)}
      className="mb-6 flex max-w-[860px] scroll-mt-6 flex-col gap-3 border-b border-border pb-6 last:border-b-0"
    >
      <h2 className="text-ui-title text-ink-strong">{title}</h2>
      {intro && <p className="-mt-1.5 text-ui-small text-ink-muted">{intro}</p>}
      {children}
    </section>
  )
}

function Field({ label, children, className }: { label?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <div className={cn('flex min-w-0 flex-1 flex-col gap-1', className)}>
      {label && <label className="text-ui-small font-medium text-ink">{label}</label>}
      {children}
    </div>
  )
}

function ToggleField({ on, onChange, children }: { on: boolean; onChange: () => void; children: ReactNode }) {
  return (
    <CheckLabel className="h-control" onToggle={onChange} control={<Switch on={on} onChange={onChange} />}>
      {children}
    </CheckLabel>
  )
}

function ReadOnlyValue({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-control items-center rounded-md border border-border bg-surface-200 px-2 font-mono text-ui-small text-ink">
      {children}
    </div>
  )
}

type TestState = { state: 'idle' } | { state: 'running' } | { state: 'ok'; ms: number } | { state: 'error'; message: string }

function FoundrySection({ s, update }: { s: Settings; update: (p: Partial<Settings>) => void }) {
  const { foundry, refreshHealth } = useApp()
  const [test, setTest] = useState<TestState>({ state: 'idle' })
  const health = foundry.state === 'ready' ? foundry.health : null
  const setDep = (i: number, patch: Partial<Settings['deployments'][number]>) =>
    update({ deployments: s.deployments.map((d, j) => (j === i ? { ...d, ...patch } : d)) })
  const num = (v: string) => Number(v.replace(/[^\d.]/g, '')) || 0

  const runTest = () => {
    setTest({ state: 'running' })
    testConnection()
      .then((r) => setTest({ state: 'ok', ms: r.latencyMs }))
      .catch((err: unknown) => setTest({ state: 'error', message: err instanceof Error ? err.message : 'Test failed.' }))
  }

  return (
    <Section
      title="Azure AI Foundry"
      intro={
        <>
          Requests go from the local server straight to your deployment. Connection settings come from{' '}
          <InlineCode>.env.local</InlineCode> and stay on the server; the API key never reaches this page.
        </>
      }
    >
      {foundry.state === 'unconfigured' && (
        <WarningBanner className="rounded-md">
          <span>
            Not configured: set <b className="font-semibold">{[...foundry.missing, ...foundry.invalid].join(', ')}</b> in{' '}
            <span className="font-mono">.env.local</span> (see <span className="font-mono">.env.example</span>), then restart{' '}
            <span className="font-mono">npm run dev</span>.
          </span>
          <span className="flex-1" />
          <Button size="xs" icon="refresh" onClick={refreshHealth}>
            Check again
          </Button>
        </WarningBanner>
      )}
      {foundry.state === 'offline' && (
        <WarningBanner className="rounded-md">
          <span>The local server is not reachable ({foundry.error}). Start the app with npm run dev.</span>
        </WarningBanner>
      )}

      <Field label="Endpoint URL">
        <ReadOnlyValue>{health?.endpoint ?? '—'}</ReadOnlyValue>
      </Field>
      <div className="flex items-end gap-4">
        <Field label="API key">
          <ReadOnlyValue>
            {health ? (
              <span className="inline-flex items-center gap-1.5 font-sans text-ink-muted">
                <Icon name="key" className="size-3.5" />
                Set in .env.local (AZURE_OPENAI_API_KEY)
              </span>
            ) : (
              '—'
            )}
          </ReadOnlyValue>
        </Field>
        <Field label="API version">
          <ReadOnlyValue>{health?.apiVersion ?? '—'}</ReadOnlyValue>
        </Field>
        <div className="flex h-control flex-none items-center gap-2">
          <Button icon="refresh" disabled={!health || test.state === 'running'} onClick={runTest}>
            {test.state === 'running' ? 'Testing…' : 'Test connection'}
          </Button>
          {test.state === 'ok' && (
            <span className="inline-flex items-center gap-1 text-ui-small text-success">
              <Icon name="check" className="size-3.5" />
              OK · {test.ms} ms
            </span>
          )}
        </div>
      </div>
      {test.state === 'error' && (
        <p role="alert" className="flex items-start gap-1.5 text-ui-small text-danger">
          <Icon name="warning" className="mt-px size-3.5" />
          {test.message}
        </p>
      )}

      <Field label="Deployments">
        <table className="w-full border-collapse rounded-md border border-border text-ui-small">
          <thead>
            <tr className="bg-surface-200 text-left text-ink-muted [&_th]:border-b [&_th]:border-border [&_th]:px-2 [&_th]:py-1.5 [&_th]:font-medium">
              <th>Default</th>
              <th>Deployment name</th>
              <th>Model</th>
              <th>Context limit (tokens)</th>
              <th>Input / 1M</th>
              <th>Output / 1M</th>
            </tr>
          </thead>
          <tbody className="[&_td]:border-b [&_td]:border-border [&_td]:px-2 [&_td]:py-1">
            {s.deployments.map((d, i) => (
              <tr key={d.name}>
                <td>
                  <Radio on={s.defaultDeployment === i} label={`Make ${d.name} the default`} onChange={() => update({ defaultDeployment: i })} />
                </td>
                <td className="font-mono whitespace-nowrap">{d.name}</td>
                <td className="font-mono whitespace-nowrap text-ink-muted">{health?.modelName ?? 'sample'}</td>
                <td>
                  <TextInput size="sm" mono value={d.limit.toLocaleString('en-US')} onChange={(e) => setDep(i, { limit: num(e.target.value) })} />
                </td>
                <td>
                  <TextInput size="sm" mono value={`$${d.inPrice.toFixed(2)}`} onChange={(e) => setDep(i, { inPrice: num(e.target.value) })} />
                </td>
                <td>
                  <TextInput size="sm" mono value={`$${d.outPrice.toFixed(2)}`} onChange={(e) => setDep(i, { outPrice: num(e.target.value) })} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <p className="text-ui-small text-ink-muted">
          {health
            ? 'The deployment comes from AZURE_OPENAI_DEPLOYMENT. '
            : 'Showing sample deployments until the server is configured. '}
          The context limit drives the context bar (amber at 75%, red at 95%).
          {health && !health.contextLimit && ' AZURE_OPENAI_CONTEXT_LIMIT is not set, so 128,000 is assumed.'}
          {health && (health.inputPricePer1M || health.outputPricePer1M)
            ? ' Prices come from AZURE_OPENAI_INPUT_PRICE_PER_1M / _OUTPUT_PRICE_PER_1M in .env.local.'
            : ''} Settings
          are saved in this browser.
        </p>
      </Field>
    </Section>
  )
}

function ExclusionsSection({ s, update }: { s: Settings; update: (p: Partial<Settings>) => void }) {
  const [draft, setDraft] = useState<string | null>(null)
  const addDraft = () => {
    const g = draft?.trim()
    if (g && !s.exclusions.includes(g)) update({ exclusions: [...s.exclusions, g] })
    setDraft(null)
  }

  return (
    <Section title="Exclusions" intro="Matched paths are shown greyed out in the explorer and are never sent.">
      <Field label={<>Glob patterns <span className="font-normal text-ink-muted">· one per chip</span></>}>
        <div className="flex flex-wrap gap-1.5 rounded-md border border-border-strong bg-surface-100 p-2">
          {s.exclusions.map((g) => (
            <span key={g} className="inline-flex h-row items-center gap-1 rounded-sm bg-surface-300 pr-1 pl-2 font-mono text-[12px] text-ink">
              {g}
              <button
                type="button"
                aria-label={`Remove ${g}`}
                onClick={() => update({ exclusions: s.exclusions.filter((x) => x !== g) })}
                className="inline-flex cursor-pointer text-ink-muted hover:text-ink"
              >
                <Icon name="close" className="size-3" />
              </button>
            </span>
          ))}
          {draft === null ? (
            <button
              type="button"
              onClick={() => setDraft('')}
              className="inline-flex h-row cursor-pointer items-center gap-1 rounded-sm border border-dashed border-border-strong px-2 text-ui-small text-accent-ink"
            >
              <Icon name="plus" className="size-3" />
              Add pattern
            </button>
          ) : (
            <input
              autoFocus
              value={draft}
              placeholder="**/*.generated.cs"
              onChange={(e) => setDraft(e.target.value)}
              onBlur={addDraft}
              onKeyDown={(e) => {
                if (e.key === 'Enter') addDraft()
                if (e.key === 'Escape') setDraft(null)
              }}
              className="h-row w-48 rounded-sm border border-focus-ring bg-surface-100 px-2 font-mono text-[12px] text-ink outline-none"
            />
          )}
        </div>
        <LinkButton className="self-start" onClick={() => update({ exclusions: defaultExclusions })}>
          Reset to defaults
        </LinkButton>
      </Field>
      <div className="flex items-end gap-4">
        <Field label="Max file size" className="flex-none">
          <TextInput
            mono
            boxClassName="w-[120px]"
            value={String(s.maxFileSizeKb)}
            onChange={(e) => update({ maxFileSizeKb: Number(e.target.value.replace(/\D/g, '')) || 0 })}
            trailing={<span className="text-ui-small text-ink-muted">KB</span>}
          />
        </Field>
        <ToggleField on={s.respectGitignore} onChange={() => update({ respectGitignore: !s.respectGitignore })}>
          Respect .gitignore
        </ToggleField>
        <ToggleField on={s.skipBinary} onChange={() => update({ skipBinary: !s.skipBinary })}>
          Skip binary files
        </ToggleField>
        <ToggleField on={s.includeHidden} onChange={() => update({ includeHidden: !s.includeHidden })}>
          Include hidden files
        </ToggleField>
      </div>
    </Section>
  )
}

/** Settings, user scope: Foundry connection, exclusion rules, output format, appearance. */
export function SettingsScreen() {
  const { settings: s, updateSettings: update, closeSettings, theme, toggleTheme, diffMode, setDiffMode } = useApp()
  const [active, setActive] = useState<string>(SECTIONS[0])
  const [scope, setScope] = useState<'user' | 'project'>('user')

  return (
    <div className="flex min-h-0 flex-1">
      <nav className="flex w-[220px] flex-none flex-col border-r border-border bg-surface-000">
        <div className="flex h-8 items-center gap-1 pr-2 pl-1.5">
          <IconButton icon="back" label="Back" onClick={closeSettings} />
          <span className="text-ui-caption text-ink-muted uppercase">Settings</span>
        </div>
        {SECTIONS.map((n) => (
          <button
            key={n}
            type="button"
            onClick={() => {
              setActive(n)
              document.getElementById(sectionId(n))?.scrollIntoView({ behavior: 'smooth' })
            }}
            className={cn(
              'flex h-[30px] cursor-pointer items-center px-4 text-left text-ui-body text-ink-muted hover:text-ink',
              active === n && 'bg-surface-selected text-ink-strong shadow-[inset_2px_0_0_var(--accent)]',
            )}
          >
            {n}
          </button>
        ))}
        <div className="mt-auto flex flex-col items-start gap-1.5 border-t border-border px-4 py-3">
          <span className="text-ui-caption text-ink-muted uppercase">Scope</span>
          <Segmented
            size="sm"
            label="Settings scope"
            value={scope}
            onChange={setScope}
            options={[
              { value: 'user', label: 'User' },
              { value: 'project', label: 'Project' },
            ]}
          />
        </div>
      </nav>

      <div className="min-w-0 flex-1 overflow-auto px-8 py-6">
        <FoundrySection s={s} update={update} />
        <ExclusionsSection s={s} update={update} />

        <Section
          title="Output format"
          intro={
            <>
              Sent as the system prompt after your enabled rules. The parser reads <InlineCode>### FILE:</InlineCode> and{' '}
              <InlineCode>### DELETE:</InlineCode> headers to build diffs.
            </>
          }
        >
          <div className="overflow-hidden rounded-md border border-border-strong">
            <div className="flex h-8 items-center gap-2 border-b border-border bg-surface-200 pr-2 pl-3">
              <span className="text-[11px] text-ink-muted">System prompt · Markdown</span>
              <span className="flex-1" />
              <span className="text-ui-small text-ink-muted">{estimateTokens(s.outputPrompt)} tokens</span>
              <Button variant="ghost" size="xs" onClick={() => update({ outputPrompt })}>
                Reset
              </Button>
            </div>
            <CodeEditor label="Output format system prompt" lang="markdown" value={s.outputPrompt} onChange={(v) => update({ outputPrompt: v })} />
          </div>
          <div className="inline-flex items-center gap-1 text-ui-small text-success">
            <Icon name="check" className="size-3.5" />
            Parser test on last response: 5 files, 0 errors
          </div>
        </Section>

        <Section title="Appearance">
          <div className="flex items-end gap-6">
            <Field label="Theme" className="flex-none">
              <Segmented<Theme>
                label="Theme"
                value={theme}
                onChange={(t) => t !== theme && toggleTheme()}
                options={[
                  { value: 'dark', label: 'Dark' },
                  { value: 'light', label: 'Light' },
                ]}
              />
            </Field>
            <Field label="Diff layout" className="flex-none">
              <Segmented<DiffMode>
                label="Diff layout"
                value={diffMode}
                onChange={setDiffMode}
                options={[
                  { value: 'unified', label: 'Unified', icon: 'unified' },
                  { value: 'split', label: 'Split', icon: 'split' },
                ]}
              />
            </Field>
          </div>
        </Section>

        <Section title="Keyboard">
          <dl className="grid max-w-[420px] grid-cols-[1fr_auto] gap-y-2 text-ui-body">
            {[
              ['Send message', 'Ctrl+Enter'],
              ['Open folder', 'Ctrl+O'],
              ['Close dialog', 'Esc'],
            ].map(([what, keys]) => (
              <div key={what} className="contents">
                <dt className="text-ink-muted">{what}</dt>
                <dd>
                  <kbd className="rounded-sm border border-border-strong px-1 font-mono text-[11px] text-ink-muted">{keys}</kbd>
                </dd>
              </div>
            ))}
          </dl>
        </Section>
      </div>
    </div>
  )
}
