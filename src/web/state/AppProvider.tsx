import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import * as sample from '@/data/sample'
import type { Change, Rule, TreeNode } from '@/data/sample'
import { fetchHealth, streamChat, type ApiMessage } from '@/lib/api'
import { parseChanges } from '@/lib/changes'
import { estimateTokens, fmtK } from '@/lib/format'
import { buildMessages, systemPrompt } from '@/lib/prompt'
import { biggestFolder, checkState, findNode, nodeStats, selectableFiles } from '@/lib/tree'
import {
  AppContext,
  type AppActions,
  type AppContextValue,
  type ChatMessage,
  type ContextMode,
  type DiffMode,
  type FoundryStatus,
  type ResponseInfo,
  type Screen,
  type Settings,
  type Theme,
  type ToastState,
  type Totals,
} from './app-context'

const DIFF_MODE_KEY = 'contextual.diffMode'
const THEME_KEY = 'contextual.theme'
/** Used for the context bar until AZURE_OPENAI_CONTEXT_LIMIT is set. */
export const FALLBACK_CONTEXT_LIMIT = 128000

function readPref<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  try {
    const v = localStorage.getItem(key)
    return v && (allowed as readonly string[]).includes(v) ? (v as T) : fallback
  } catch {
    return fallback
  }
}

function writePref(key: string, value: string) {
  try {
    localStorage.setItem(key, value)
  } catch {
    // Preferences are a convenience; ignore storage failures.
  }
}

function now(): string {
  return new Date().toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' })
}

/** With "Respect .gitignore" off, files excluded only by .gitignore become selectable. */
function applyGitignore(nodes: TreeNode[], respect: boolean): TreeNode[] {
  if (respect) return nodes
  return nodes.map((n) => ({
    ...n,
    excluded: n.excluded === '.gitignore' ? undefined : n.excluded,
    children: n.children && applyGitignore(n.children, respect),
  }))
}

function withStale(nodes: TreeNode[], stale: ReadonlySet<string>): TreeNode[] {
  return nodes.map((n) => ({
    ...n,
    changedOnDisk: stale.has(n.path),
    children: n.children && withStale(n.children, stale),
  }))
}

const includeFor = (changes: Change[]) => Object.fromEntries(changes.map((c) => [c.path, !c.conflict]))

const initialSettings: Settings = {
  deployments: sample.deployments,
  defaultDeployment: 0,
  exclusions: sample.defaultExclusions,
  maxFileSizeKb: 256,
  respectGitignore: true,
  skipBinary: true,
  includeHidden: false,
  outputPrompt: sample.outputPrompt,
}

const initialMessages: ChatMessage[] = [
  { id: 'u1', role: 'user', text: sample.userPrompt, time: '14:31', meta: '12 files · 3 rules · full context' },
  {
    id: 'a1',
    role: 'assistant',
    text: sample.assistantResponse,
    time: '14:32',
    model: 'gpt-4.1',
    meta: '18.4k in / 2.1k out · $0.054',
    changes: sample.changes,
  },
]

const initialStale = new Set(['src/Inventory.Api/Endpoints/OrderEndpoints.cs'])

let nextId = 0
const newId = (prefix: string) => `${prefix}${Date.now().toString(36)}${(nextId++).toString(36)}`

export function AppProvider({ children }: { children: ReactNode }) {
  const [screen, setScreen] = useState<Screen>('welcome')
  const [settingsReturn, setSettingsReturn] = useState<Screen>('welcome')
  const [theme, setTheme] = useState<Theme>(() => readPref(THEME_KEY, ['dark', 'light'] as const, 'dark'))

  const [selected, setSelected] = useState<ReadonlySet<string>>(() => new Set(sample.initialSelection))
  const [open, setOpen] = useState<ReadonlySet<string>>(() => new Set(sample.initialOpen))
  const [stale, setStale] = useState<ReadonlySet<string>>(initialStale)
  const [filter, setFilter] = useState('')

  const [rules, setRules] = useState<Rule[]>(sample.rules)
  const [rulesOn, setRulesOn] = useState<ReadonlySet<string>>(() => new Set(sample.initialRulesOn))
  const [rulePreview, setRulePreview] = useState<string | null>(null)

  const [foundry, setFoundry] = useState<FoundryStatus>({ state: 'checking' })
  const [deployment, setDeployment] = useState(0)
  const [contextMode, setContextMode] = useState<ContextMode>('full')
  const [messages, setMessages] = useState<ChatMessage[]>(initialMessages)
  const abortRef = useRef<AbortController | null>(null)

  const [previewTabs, setPreviewTabs] = useState<string[]>(['src/Inventory.Domain/Orders/Order.cs'])
  const [activeTab, setActiveTab] = useState<'chat' | string>('chat')
  const [centerView, setCenterView] = useState<'chat' | 'diff'>('chat')
  const [diffMode, setDiffModeState] = useState<DiffMode>(() =>
    readPref(DIFF_MODE_KEY, ['unified', 'split'] as const, 'unified'),
  )
  const [diffFocus, setDiffFocus] = useState<string | null>(null)

  const [include, setInclude] = useState<Record<string, boolean>>(() => includeFor(sample.changes))
  const [applyOpen, setApplyOpen] = useState(false)
  const [toast, setToast] = useState<ToastState | null>(null)

  const [settings, setSettings] = useState<Settings>(initialSettings)

  useEffect(() => {
    const root = document.documentElement
    root.classList.toggle('dark', theme === 'dark')
    root.classList.toggle('light', theme === 'light')
  }, [theme])

  // Ask the local server whether Foundry is configured; use its deployment when it is.
  const refreshHealth = useCallback(() => {
    const ctrl = new AbortController()
    fetchHealth(ctrl.signal)
      .then((h) => {
        if (!h.configured) {
          setFoundry({ state: 'unconfigured', missing: h.missing, invalid: h.invalid })
          return
        }
        setFoundry({ state: 'ready', health: h })
        setSettings((s) => ({
          ...s,
          defaultDeployment: 0,
          deployments: [
            {
              name: h.deployment,
              limit: h.contextLimit ?? FALLBACK_CONTEXT_LIMIT,
              inPrice: 0,
              outPrice: 0,
            },
          ],
        }))
        setDeployment(0)
      })
      .catch((err: unknown) => {
        if (ctrl.signal.aborted) return
        setFoundry({ state: 'offline', error: err instanceof Error ? err.message : 'Server unavailable.' })
      })
    return ctrl
  }, [])

  useEffect(() => {
    const ctrl = refreshHealth()
    return () => ctrl.abort()
  }, [refreshHealth])

  // Deep links to the design-system states, for reviewing the mockup:
  // #workspace #preview #heavy #diff #split #apply #applied #settings, plus #light.
  useEffect(() => {
    const tags = new Set(window.location.hash.slice(1).split(/[,+]/).filter(Boolean))
    if (!tags.size) return
    if (tags.has('light')) setTheme('light')
    if (tags.has('settings')) setScreen('settings')
    else if ([...tags].some((t) => t !== 'light')) setScreen('workspace')
    if (tags.has('preview') || tags.has('heavy')) {
      setActiveTab('src/Inventory.Domain/Orders/Order.cs')
      setDeployment(1)
      setRulesOn((s) => new Set(s).add('CLAUDE.md'))
      setSelected((s) => {
        const next = new Set(s)
        for (const f of selectableFiles(sample.tree).filter((p) => p.startsWith('src/Inventory.Infrastructure/')))
          next.add(f)
        return next
      })
      setOpen((s) => new Set(s).add('src/Inventory.Infrastructure').add('src/Inventory.Infrastructure/Migrations'))
    }
    if (tags.has('diff') || tags.has('split')) setCenterView('diff')
    if (tags.has('split')) setDiffModeState('split')
    if (tags.has('apply')) setApplyOpen(true)
    if (tags.has('applied'))
      setToast({ title: '4 files written', detail: 'Backup saved · 1 deleted, 2 created, 1 overwritten', undoable: true })
  }, [])

  const tree = useMemo(
    () => withStale(applyGitignore(sample.tree, settings.respectGitignore), stale),
    [settings.respectGitignore, stale],
  )

  // The Changes panel follows the latest response that proposed file changes.
  const { changes, response } = useMemo(() => {
    let number = 0
    let last: { changes: Change[]; response: ResponseInfo } | null = null
    for (const m of messages) {
      if (m.role !== 'assistant') continue
      number++
      if (m.changes?.length && !m.streaming)
        last = { changes: m.changes, response: { id: m.id, number, time: m.time, model: m.model ?? '' } }
    }
    return last ?? { changes: [] as Change[], response: null }
  }, [messages])

  const streaming = messages.some((m) => m.streaming)

  const totals = useMemo<Totals>(() => {
    let files = 0
    let fileTokens = 0
    for (const n of tree) {
      const s = nodeStats(n, selected)
      files += s.selected
      fileTokens += s.selectedTokens
    }
    const onRules = rules.filter((r) => rulesOn.has(r.path))
    const ruleTokens = onRules.reduce((a, r) => a + r.tokens, 0)
    const raw =
      fileTokens + ruleTokens + sample.PROMPT_TOKENS + sample.USER_MESSAGE_TOKENS + sample.REPLY_TOKENS
    const tokens = Math.round(raw / 100) * 100
    const dep = settings.deployments[deployment] ?? settings.deployments[0]
    const share = tokens / dep.limit
    const level = share >= 0.95 ? 'danger' : share >= 0.75 ? 'warn' : 'ok'
    const cost = (tokens / 1e6) * dep.inPrice + (sample.EST_OUTPUT_TOKENS / 1e6) * dep.outPrice
    const big = level === 'ok' ? null : biggestFolder(tree, selected)
    return {
      files,
      rules: onRules.length,
      fileTokens,
      ruleTokens,
      tokens,
      share,
      level,
      cost,
      hint: big ? `${big.name}: ${fmtK(big.tokens)}` : null,
    }
  }, [tree, selected, rules, rulesOn, settings.deployments, deployment])

  const toggleSet = useCallback(
    <T,>(setter: (fn: (s: ReadonlySet<T>) => ReadonlySet<T>) => void, value: T) =>
      setter((s) => {
        const next = new Set(s)
        if (next.has(value)) next.delete(value)
        else next.add(value)
        return next
      }),
    [],
  )

  const updateMessage = useCallback(
    (id: string, fn: (m: ChatMessage) => ChatMessage) => setMessages((ms) => ms.map((m) => (m.id === id ? fn(m) : m))),
    [],
  )

  const send = useCallback(
    (text: string) => {
      if (abortRef.current) return
      const dep = settings.deployments[deployment] ?? settings.deployments[0]
      const time = now()
      const full = contextMode === 'full'

      // "Resend full context" attaches the current files and drops older copies from history;
      // "Only send new message" keeps what earlier messages carried.
      const history: ApiMessage[] = messages
        .filter((m) => (m.role === 'user' || m.role === 'assistant') && !m.streaming)
        .map((m) => ({ role: m.role as 'user' | 'assistant', content: m.role === 'user' && !full ? (m.sent ?? m.text) : m.text }))
      const files = full ? [...selected].sort().map((path) => ({ path, content: sample.fileContents[path] })) : null
      const onRules = rules.filter((r) => rulesOn.has(r.path))
      const apiMessages = buildMessages({ system: systemPrompt(onRules, settings.outputPrompt), history, text, files })
      const sent = apiMessages[apiMessages.length - 1].content

      const userId = newId('u')
      const aiId = newId('a')
      setMessages((ms) => [
        ...ms,
        {
          id: userId,
          role: 'user',
          text,
          sent,
          time,
          meta: `${totals.files} files · ${totals.rules} rules · ${full ? 'full context' : 'new message only'}`,
        },
        { id: aiId, role: 'assistant', text: '', time, model: dep.name, streaming: true },
      ])

      const ctrl = new AbortController()
      abortRef.current = ctrl
      let reply = ''
      let usage: { inputTokens: number; outputTokens: number } | null = null
      let failure: string | null = null

      streamChat(
        apiMessages,
        (e) => {
          if (e.type === 'delta') {
            reply += e.text
            updateMessage(aiId, (m) => ({ ...m, text: reply }))
          } else if (e.type === 'usage') usage = e
          else if (e.type === 'error') failure = e.message
        },
        ctrl.signal,
      )
        .catch((err: unknown) => {
          failure = ctrl.signal.aborted ? 'Stopped.' : err instanceof Error ? err.message : 'Request failed.'
        })
        .finally(() => {
          abortRef.current = null
          const u = usage as { inputTokens: number; outputTokens: number } | null
          const priced = dep.inPrice > 0 || dep.outPrice > 0
          const meta = u
            ? `${fmtK(u.inputTokens)} in / ${fmtK(u.outputTokens)} out` +
              (priced ? ` · $${((u.inputTokens / 1e6) * dep.inPrice + (u.outputTokens / 1e6) * dep.outPrice).toFixed(3)}` : '')
            : undefined
          const proposed = parseChanges(reply, {
            exists: (p) => !!findNode(sample.tree, p),
            original: (p) => sample.fileContents[p],
            changedOnDisk: (p) => stale.has(p),
          })

          setMessages((ms) => {
            const next = ms.flatMap<ChatMessage>((m) => {
              if (m.id !== aiId) return [m]
              if (!reply) return [] // Nothing came back; only the notice below remains.
              return [{ ...m, streaming: false, meta, changes: proposed }]
            })
            if (failure)
              next.push({ id: newId('n'), role: 'note', text: failure, time: now(), tone: failure === 'Stopped.' ? 'warn' : 'error' })
            return next
          })
          if (proposed.length) setInclude(includeFor(proposed))
        })
    },
    [settings, deployment, contextMode, messages, selected, rules, rulesOn, totals, stale, updateMessage],
  )

  const actions = useMemo<AppActions>(
    () => ({
      openProject: () => setScreen('workspace'),
      closeProject: () => setScreen('welcome'),
      openSettings: () => {
        if (screen !== 'settings') setSettingsReturn(screen)
        setScreen('settings')
      },
      closeSettings: () => setScreen(settingsReturn),
      toggleTheme: () =>
        setTheme((t) => {
          const next = t === 'dark' ? 'light' : 'dark'
          writePref(THEME_KEY, next)
          return next
        }),

      toggleFile: (path) => toggleSet(setSelected, path),
      toggleFolder: (node) =>
        setSelected((s) => {
          const next = new Set(s)
          const files = selectableFiles([node])
          if (checkState(node, s) === 'on') files.forEach((f) => next.delete(f))
          else files.forEach((f) => next.add(f))
          return next
        }),
      selectAll: () => setSelected(new Set(selectableFiles(tree))),
      clearSelection: () => setSelected(new Set()),
      toggleOpen: (path) => toggleSet(setOpen, path),
      setFilter,
      refreshStale: () => setStale(new Set()),

      toggleRule: (path) => toggleSet(setRulesOn, path),
      previewRule: setRulePreview,
      importRule: (file) => {
        void file.text().then((content) => {
          const path = file.name
          const type = /SKILL\.md$/i.test(path) ? 'Skill' : 'Rule'
          setRules((rs) => [...rs.filter((r) => r.path !== path), { path, type, tokens: estimateTokens(content), content }])
          setRulesOn((s) => new Set(s).add(path))
          setRulePreview(path)
        })
      },

      setDeployment,
      setContextMode,
      send,
      stop: () => abortRef.current?.abort(),
      refreshHealth: () => void refreshHealth(),

      openPreview: (path) => {
        setPreviewTabs((tabs) => (tabs.includes(path) ? tabs : [...tabs, path]))
        setActiveTab(path)
        setCenterView('chat')
      },
      closeTab: (path) => {
        setPreviewTabs((tabs) => tabs.filter((t) => t !== path))
        setActiveTab((a) => (a === path ? 'chat' : a))
      },
      setActiveTab,
      openDiff: (focus) => {
        setDiffFocus(focus ?? null)
        setCenterView('diff')
      },
      backToChat: () => setCenterView('chat'),
      setDiffMode: (m) => {
        writePref(DIFF_MODE_KEY, m)
        setDiffModeState(m)
      },

      toggleInclude: (path) => setInclude((i) => ({ ...i, [path]: !i[path] })),
      setIncludeAll: (on) => setInclude(Object.fromEntries(changes.map((c) => [c.path, on && !c.conflict]))),
      openApply: () => setApplyOpen(true),
      closeApply: () => setApplyOpen(false),
      apply: () => {
        const picked = changes.filter((c) => include[c.path])
        const n = (s: string) => picked.filter((c) => c.status === s).length
        const parts = [n('D') && `${n('D')} deleted`, n('A') && `${n('A')} created`, n('M') && `${n('M')} overwritten`].filter(
          Boolean,
        )
        setApplyOpen(false)
        setToast({
          title: `${picked.length} ${picked.length === 1 ? 'file' : 'files'} written`,
          detail: `Backup saved · ${parts.join(', ')}`,
          undoable: true,
        })
      },
      undoApply: () => setToast({ title: 'Changes undone', detail: 'Restored from .contextual/backups', undoable: false }),
      dismissToast: () => setToast(null),

      updateSettings: (patch) => setSettings((s) => ({ ...s, ...patch })),
    }),
    [toggleSet, tree, include, screen, settingsReturn, send, refreshHealth, changes],
  )

  const value: AppContextValue = {
    screen,
    theme,
    tree,
    selected,
    open,
    stale,
    filter,
    rules,
    rulesOn,
    rulePreview,
    foundry,
    deployment,
    contextMode,
    messages,
    streaming,
    previewTabs,
    activeTab,
    centerView,
    diffMode,
    diffFocus,
    changes,
    response,
    include,
    applyOpen,
    toast,
    settings,
    totals,
    ...actions,
  }

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>
}
