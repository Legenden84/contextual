import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import * as sample from '@/data/sample'
import type { Change, Rule, TreeNode } from '@/data/sample'
import * as api from '@/lib/api'
import { parseChanges } from '@/lib/changes'
import { estimateTokens, fmtK } from '@/lib/format'
import { parseMarkdown } from '@/lib/markdown'
import { buildMessages, systemPrompt, type ContextFile } from '@/lib/prompt'
import { biggestFolder, checkState, findNode, nodeStats, selectableFiles } from '@/lib/tree'
import {
  AppContext,
  type AppActions,
  type AppContextValue,
  type ChatMessage,
  type ContextMode,
  type DiffMode,
  type FoundryStatus,
  type Project,
  type RecentProject,
  type ResponseInfo,
  type Screen,
  type Settings,
  type Theme,
  type ToastState,
  type Totals,
} from './app-context'

const DIFF_MODE_KEY = 'contextual.diffMode'
const THEME_KEY = 'contextual.theme'
const RECENTS_KEY = 'contextual.recentProjects'
/** Used for the context bar until AZURE_OPENAI_CONTEXT_LIMIT is set. */
export const FALLBACK_CONTEXT_LIMIT = 128000
/** Don't rescan on window focus more often than this. */
const FOCUS_RESCAN_MS = 5000

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

function readRecents(): RecentProject[] {
  try {
    const list = JSON.parse(localStorage.getItem(RECENTS_KEY) ?? '[]') as unknown
    return Array.isArray(list) ? (list as RecentProject[]).filter((r) => r && typeof r.path === 'string') : []
  } catch {
    return []
  }
}

function now(): string {
  return new Date().toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' })
}

const errorText = (err: unknown, fallback: string) => (err instanceof Error ? err.message : fallback)

/** With "Respect .gitignore" off, files excluded only by .gitignore become selectable (sample only; folders rescan). */
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

function fileMtimes(nodes: TreeNode[], out = new Map<string, number>()): Map<string, number> {
  for (const n of nodes) {
    if (n.kind === 'file' && n.mtimeMs != null) out.set(n.path, n.mtimeMs)
    if (n.children) fileMtimes(n.children, out)
  }
  return out
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

const sampleMessages: ChatMessage[] = [
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

const sampleProject: Project = {
  kind: 'sample',
  name: sample.project.name,
  root: sample.project.path,
  branch: sample.project.branch,
  truncated: false,
}

let nextId = 0
const newId = (prefix: string) => `${prefix}${Date.now().toString(36)}${(nextId++).toString(36)}`

export function AppProvider({ children }: { children: ReactNode }) {
  const [screen, setScreen] = useState<Screen>('welcome')
  const [settingsReturn, setSettingsReturn] = useState<Screen>('welcome')
  const [theme, setTheme] = useState<Theme>(() => readPref(THEME_KEY, ['dark', 'light'] as const, 'dark'))

  const [project, setProject] = useState<Project | null>(null)
  const [rawTree, setRawTree] = useState<TreeNode[]>([])
  const [recents, setRecents] = useState<RecentProject[]>(readRecents)
  const [folderDialogOpen, setFolderDialogOpen] = useState(false)
  const [scanning, setScanning] = useState(false)
  /** Contents read from disk for the open folder, by project path. */
  const contents = useRef(new Map<string, string>())
  /** Modified time of each file when it was last sent to the model. */
  const sentMtimes = useRef(new Map<string, number>())

  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set())
  const [open, setOpen] = useState<ReadonlySet<string>>(new Set())
  const [stale, setStale] = useState<ReadonlySet<string>>(new Set())
  const [filter, setFilter] = useState('')

  const [rules, setRules] = useState<Rule[]>([])
  const [rulesOn, setRulesOn] = useState<ReadonlySet<string>>(new Set())
  const [rulePreview, setRulePreview] = useState<string | null>(null)

  const [foundry, setFoundry] = useState<FoundryStatus>({ state: 'checking' })
  const [deployment, setDeployment] = useState(0)
  const [contextMode, setContextMode] = useState<ContextMode>('full')
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const abortRef = useRef<AbortController | null>(null)

  const [previewTabs, setPreviewTabs] = useState<string[]>([])
  const [activeTab, setActiveTab] = useState<'chat' | string>('chat')
  const [centerView, setCenterView] = useState<'chat' | 'diff'>('chat')
  const [diffMode, setDiffModeState] = useState<DiffMode>(() =>
    readPref(DIFF_MODE_KEY, ['unified', 'split'] as const, 'unified'),
  )
  const [diffFocus, setDiffFocus] = useState<string | null>(null)

  const [include, setInclude] = useState<Record<string, boolean>>({})
  const [applyOpen, setApplyOpen] = useState(false)
  const [toast, setToast] = useState<ToastState | null>(null)

  const [settings, setSettings] = useState<Settings>(initialSettings)

  useEffect(() => {
    const root = document.documentElement
    root.classList.toggle('dark', theme === 'dark')
    root.classList.toggle('light', theme === 'light')
  }, [theme])

  // ---------------------------------------------------------------- Foundry status

  const refreshHealth = useCallback(() => {
    const ctrl = new AbortController()
    api
      .fetchHealth(ctrl.signal)
      .then((h) => {
        if (!h.configured) {
          setFoundry({ state: 'unconfigured', missing: h.missing, invalid: h.invalid })
          return
        }
        setFoundry({ state: 'ready', health: h })
        setSettings((s) => ({
          ...s,
          defaultDeployment: 0,
          deployments: [{ name: h.deployment, limit: h.contextLimit ?? FALLBACK_CONTEXT_LIMIT, inPrice: 0, outPrice: 0 }],
        }))
        setDeployment(0)
      })
      .catch((err: unknown) => {
        if (!ctrl.signal.aborted) setFoundry({ state: 'offline', error: errorText(err, 'Server unavailable.') })
      })
    return ctrl
  }, [])

  useEffect(() => {
    const ctrl = refreshHealth()
    return () => ctrl.abort()
  }, [refreshHealth])

  // ---------------------------------------------------------------- opening projects

  const resetWorkspace = useCallback(() => {
    contents.current.clear()
    sentMtimes.current.clear()
    setFilter('')
    setRulePreview(null)
    setActiveTab('chat')
    setCenterView('chat')
    setDiffFocus(null)
    setApplyOpen(false)
    setToast(null)
  }, [])

  const loadSample = useCallback(() => {
    resetWorkspace()
    setProject(sampleProject)
    setRawTree(sample.tree)
    setSelected(new Set(sample.initialSelection))
    setOpen(new Set(sample.initialOpen))
    setStale(new Set(['src/Inventory.Api/Endpoints/OrderEndpoints.cs']))
    setRules(sample.rules)
    setRulesOn(new Set(sample.initialRulesOn))
    setMessages(sampleMessages)
    setPreviewTabs(['src/Inventory.Domain/Orders/Order.cs'])
    setInclude(includeFor(sample.changes))
    setScreen('workspace')
  }, [resetWorkspace])

  const scanOptions = useMemo<api.ScanOptions>(
    () => ({
      exclusions: settings.exclusions,
      maxFileSizeKb: settings.maxFileSizeKb,
      respectGitignore: settings.respectGitignore,
      skipBinary: settings.skipBinary,
      includeHidden: settings.includeHidden,
    }),
    [settings.exclusions, settings.maxFileSizeKb, settings.respectGitignore, settings.skipBinary, settings.includeHidden],
  )
  const scanKey = JSON.stringify(scanOptions)
  const lastScanKey = useRef('')

  const openFolder = useCallback(
    async (path: string): Promise<string | null> => {
      try {
        setScanning(true)
        const r = await api.scanProject(path, scanOptions)
        lastScanKey.current = JSON.stringify(scanOptions)
        resetWorkspace()
        setProject({ kind: 'folder', name: r.name, root: r.root, branch: r.branch, truncated: r.truncated })
        setRawTree(r.tree)
        setSelected(new Set())
        setOpen(new Set())
        setStale(new Set())
        setRules(r.rules)
        setRulesOn(new Set(r.rules.filter((x) => x.type === 'Rule').map((x) => x.path)))
        setMessages([])
        setPreviewTabs([])
        setInclude({})
        setFolderDialogOpen(false)
        setScreen('workspace')
        setRecents((list) => {
          const next = [{ name: r.name, path: r.root, openedAt: new Date().toISOString() }, ...list.filter((x) => x.path !== r.root)].slice(0, 8)
          writePref(RECENTS_KEY, JSON.stringify(next))
          return next
        })
        return null
      } catch (err) {
        return errorText(err, 'Could not open the folder.')
      } finally {
        setScanning(false)
      }
    },
    [scanOptions, resetWorkspace],
  )

  /** Re-reads the open folder: new/removed files, exclusions, and files changed since they were sent. */
  const rescan = useCallback(async () => {
    if (project?.kind !== 'folder') return
    try {
      setScanning(true)
      const r = await api.scanProject(project.root, scanOptions)
      lastScanKey.current = JSON.stringify(scanOptions)
      const mtimes = fileMtimes(r.tree)
      const changed = new Set<string>()
      for (const [path, sentAt] of sentMtimes.current) if (mtimes.get(path) !== sentAt) changed.add(path)
      for (const [path] of contents.current) if (!sentMtimes.current.has(path)) contents.current.delete(path)

      setRawTree(r.tree)
      setProject((p) => (p ? { ...p, branch: r.branch, truncated: r.truncated } : p))
      const selectable = new Set(selectableFiles(r.tree))
      setSelected((s) => new Set([...s].filter((p) => selectable.has(p))))
      setStale(changed)
      setRules((old) => {
        const content = new Map(old.map((x) => [x.path, x.content]))
        setRulesOn((on) => new Set([...on].filter((p) => r.rules.some((x) => x.path === p))))
        return r.rules.map((x) => ({ ...x, content: x.content ?? content.get(x.path) ?? '' }))
      })
    } catch (err) {
      setToast({ title: 'Could not rescan the folder', detail: errorText(err, 'Scan failed.'), undoable: false, tone: 'error' })
    } finally {
      setScanning(false)
    }
  }, [project, scanOptions])

  // Exclusion settings changed: rescan the open folder.
  useEffect(() => {
    if (project?.kind !== 'folder' || scanKey === lastScanKey.current) return
    const t = window.setTimeout(() => void rescan(), 400)
    return () => window.clearTimeout(t)
  }, [scanKey, project, rescan])

  // Coming back to the window: pick up files changed in the editor.
  const lastFocusScan = useRef(0)
  useEffect(() => {
    if (project?.kind !== 'folder') return
    const onFocus = () => {
      if (Date.now() - lastFocusScan.current < FOCUS_RESCAN_MS) return
      lastFocusScan.current = Date.now()
      void rescan()
    }
    window.addEventListener('focus', onFocus)
    return () => window.removeEventListener('focus', onFocus)
  }, [project, rescan])

  const loadFile = useCallback(
    async (path: string): Promise<api.FileContent> => {
      if (project?.kind === 'sample') {
        const content = sample.fileContents[path]
        return content != null ? { path, content } : { path, error: 'The sample project has no contents for this file.' }
      }
      if (!project) return { path, error: 'No project is open.' }
      const cached = contents.current.get(path)
      if (cached != null) return { path, content: cached }
      try {
        const [f] = await api.readFiles(project.root, [path])
        if (f.content != null) contents.current.set(path, f.content)
        return f
      } catch (err) {
        return { path, error: errorText(err, 'Could not read the file.') }
      }
    },
    [project],
  )

  // #open=/path/to/project opens that folder directly (bookmarkable).
  const openedFromHash = useRef(false)
  useEffect(() => {
    const m = /^#open=(.+)$/.exec(window.location.hash)
    if (!m || openedFromHash.current) return
    openedFromHash.current = true
    void openFolder(decodeURIComponent(m[1])).then((err) => {
      if (err) setToast({ title: 'Could not open the folder', detail: err, undoable: false, tone: 'error' })
    })
  }, [openFolder])

  // Deep links to the design-system states, for reviewing the mockup (sample project):
  // #workspace #preview #heavy #diff #split #apply #applied #settings, plus #light.
  useEffect(() => {
    if (window.location.hash.startsWith('#open=')) return
    const tags = new Set(window.location.hash.slice(1).split(/[,+]/).filter(Boolean))
    if (!tags.size) return
    if (tags.has('light')) setTheme('light')
    if (![...tags].some((t) => t !== 'light')) return
    loadSample()
    if (tags.has('settings')) setScreen('settings')
    if (tags.has('preview') || tags.has('heavy')) {
      setActiveTab('src/Inventory.Domain/Orders/Order.cs')
      setDeployment(1)
      setRulesOn((s) => new Set(s).add('CLAUDE.md'))
      setSelected((s) => {
        const next = new Set(s)
        for (const f of selectableFiles(sample.tree).filter((p) => p.startsWith('src/Inventory.Infrastructure/'))) next.add(f)
        return next
      })
      setOpen((s) => new Set(s).add('src/Inventory.Infrastructure').add('src/Inventory.Infrastructure/Migrations'))
    }
    if (tags.has('diff') || tags.has('split')) setCenterView('diff')
    if (tags.has('split')) setDiffModeState('split')
    if (tags.has('apply')) setApplyOpen(true)
    if (tags.has('applied'))
      setToast({ title: '4 files written', detail: 'Backup saved · 1 deleted, 2 created, 1 overwritten', undoable: true })
  }, [loadSample])

  // ---------------------------------------------------------------- derived state

  const tree = useMemo(
    () => withStale(project?.kind === 'folder' ? rawTree : applyGitignore(rawTree, settings.respectGitignore), stale),
    [project, rawTree, settings.respectGitignore, stale],
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
    const history = messages.reduce((a, m) => a + (m.role === 'note' ? 0 : estimateTokens(m.text)), 0)
    const raw = fileTokens + ruleTokens + estimateTokens(settings.outputPrompt) + sample.USER_MESSAGE_TOKENS + history
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
  }, [tree, selected, rules, rulesOn, messages, settings.deployments, settings.outputPrompt, deployment])

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

  const addNote = useCallback(
    (text: string, tone: 'warn' | 'error') =>
      setMessages((ms) => [...ms, { id: newId('n'), role: 'note', text, time: now(), tone }]),
    [],
  )

  // ---------------------------------------------------------------- chat

  const send = useCallback(
    (text: string) => {
      if (abortRef.current || !project) return
      const ctrl = new AbortController()
      abortRef.current = ctrl
      const dep = settings.deployments[deployment] ?? settings.deployments[0]
      const full = contextMode === 'full'
      const folder = project.kind === 'folder'

      void (async () => {
        // Read the selected files now, so the model sees what is on disk at this moment.
        let files: ContextFile[] | null = null
        if (full) {
          const paths = [...selected].sort()
          if (folder) {
            try {
              const read = await api.readFiles(project.root, paths)
              for (const f of read) {
                if (f.content == null) continue
                contents.current.set(f.path, f.content)
                if (f.mtimeMs != null) sentMtimes.current.set(f.path, f.mtimeMs)
              }
              files = read.map((f) => ({ path: f.path, content: f.content }))
              setStale((s) => new Set([...s].filter((p) => !paths.includes(p))))
            } catch (err) {
              abortRef.current = null
              addNote(`Not sent: could not read the selected files. ${errorText(err, '')}`, 'error')
              return
            }
          } else files = paths.map((path) => ({ path, content: sample.fileContents[path] }))
        }

        // "Resend full context" attaches the current files and drops older copies from history;
        // "Only send new message" keeps what earlier messages carried.
        const history: api.ApiMessage[] = messages
          .filter((m) => (m.role === 'user' || m.role === 'assistant') && !m.streaming)
          .map((m) => ({
            role: m.role as 'user' | 'assistant',
            content: m.role === 'user' && !full ? (m.sent ?? m.text) : m.text,
          }))
        const onRules = rules.filter((r) => rulesOn.has(r.path))
        const apiMessages = buildMessages({ system: systemPrompt(onRules, settings.outputPrompt), history, text, files })
        const sent = apiMessages[apiMessages.length - 1].content

        const time = now()
        const aiId = newId('a')
        setMessages((ms) => [
          ...ms,
          {
            id: newId('u'),
            role: 'user',
            text,
            sent,
            time,
            meta: `${totals.files} files · ${totals.rules} rules · ${full ? 'full context' : 'new message only'}`,
          },
          { id: aiId, role: 'assistant', text: '', time, model: dep.name, streaming: true },
        ])

        // Collected from the stream; an object so the callbacks don't reassign locals.
        const out = {
          reply: '',
          usage: null as { inputTokens: number; outputTokens: number } | null,
          finish: null as string | null,
          failure: null as string | null,
        }
        try {
          await api.streamChat(
            apiMessages,
            (e) => {
              if (e.type === 'delta') {
                out.reply += e.text
                const text = out.reply
                updateMessage(aiId, (m) => ({ ...m, text }))
              } else if (e.type === 'usage') out.usage = e
              else if (e.type === 'done') out.finish = e.finishReason
              else if (e.type === 'error') out.failure = e.message
            },
            ctrl.signal,
          )
        } catch (err) {
          out.failure = ctrl.signal.aborted ? 'Stopped.' : errorText(err, 'Request failed.')
        }
        abortRef.current = null

        // Originals for every file the reply touches, so diffs compare against disk.
        const { reply, usage: u, finish, failure } = out
        const touched = parseMarkdown(reply)
          .map((b) => (b.kind === 'code' ? b.path : b.kind === 'delete' ? b.path : undefined))
          .filter((p): p is string => !!p && !!findNode(tree, p))
        const missing = folder ? touched.filter((p) => !contents.current.has(p)) : []
        if (missing.length) {
          try {
            for (const f of await api.readFiles(project.root, missing)) if (f.content != null) contents.current.set(f.path, f.content)
          } catch {
            // Diffs for those files will show the whole file as added.
          }
        }
        const truncated = finish === 'length'
        const proposed = truncated
          ? []
          : parseChanges(reply, {
              exists: (p) => !!findNode(tree, p),
              original: (p) => (folder ? contents.current.get(p) : sample.fileContents[p]),
              changedOnDisk: (p) => stale.has(p),
            })

        const priced = dep.inPrice > 0 || dep.outPrice > 0
        const meta = u
          ? `${fmtK(u.inputTokens)} in / ${fmtK(u.outputTokens)} out` +
            (priced ? ` · $${((u.inputTokens / 1e6) * dep.inPrice + (u.outputTokens / 1e6) * dep.outPrice).toFixed(3)}` : '')
          : undefined

        setMessages((ms) => {
          const next = ms.flatMap<ChatMessage>((m) => {
            if (m.id !== aiId) return [m]
            if (!reply) return []
            return [{ ...m, streaming: false, meta, changes: proposed }]
          })
          const note = (t: string, tone: 'warn' | 'error') => next.push({ id: newId('n'), role: 'note', text: t, time: now(), tone })
          if (failure) note(failure, failure === 'Stopped.' ? 'warn' : 'error')
          if (truncated)
            note(
              'The reply hit the completion token limit and was cut off, so its file changes were not offered for apply. Raise AZURE_OPENAI_MAX_COMPLETION_TOKENS or ask for fewer files.',
              'warn',
            )
          return next
        })
        if (proposed.length) setInclude(includeFor(proposed))
      })()
    },
    [project, settings, deployment, contextMode, messages, selected, rules, rulesOn, totals, stale, tree, updateMessage, addNote],
  )

  // ---------------------------------------------------------------- apply

  const apply = useCallback(
    async (backup: boolean): Promise<string | null> => {
      const picked = changes.filter((c) => include[c.path])
      if (!picked.length || !project) return null
      const count = (s: string) => picked.filter((c) => c.status === s).length
      const parts = [count('D') && `${count('D')} deleted`, count('A') && `${count('A')} created`, count('M') && `${count('M')} overwritten`].filter(Boolean)
      const title = `${picked.length} ${picked.length === 1 ? 'file' : 'files'} saved`

      if (project.kind === 'sample') {
        setApplyOpen(false)
        setToast({ title, detail: `Sample project, nothing written · ${parts.join(', ')}`, undoable: true })
        return null
      }
      try {
        const r = await api.applyChanges(
          project.root,
          picked.map((c) => ({
            path: c.path,
            status: c.status,
            content: c.status === 'D' ? undefined : c.new,
            expected: c.status === 'A' ? undefined : c.old,
          })),
          backup,
        )
        for (const c of picked) {
          sentMtimes.current.delete(c.path)
          if (c.status === 'D') contents.current.delete(c.path)
          else contents.current.set(c.path, c.new)
        }
        setApplyOpen(false)
        setToast({
          title,
          detail: `${parts.join(', ')} · not committed${project.branch ? ` on ${project.branch}` : ''}`,
          undoable: !!r.backupId,
          backupId: r.backupId ?? undefined,
        })
        void rescan()
        return null
      } catch (err) {
        return errorText(err, 'Could not write the files.')
      }
    },
    [changes, include, project, rescan],
  )

  const undoApply = useCallback(() => {
    if (project?.kind !== 'folder' || !toast?.backupId) {
      setToast({ title: 'Changes undone', detail: 'Restored the previous versions', undoable: false })
      return
    }
    const id = toast.backupId
    api
      .undoApply(project.root, id)
      .then((r) => {
        contents.current.clear()
        setToast({ title: 'Changes undone', detail: `Restored ${r.restored}, removed ${r.removed}`, undoable: false })
        void rescan()
      })
      .catch((err: unknown) => setToast({ title: 'Undo failed', detail: errorText(err, 'Could not restore.'), undoable: false, tone: 'error' }))
  }, [project, toast, rescan])

  // ---------------------------------------------------------------- actions

  const actions = useMemo<AppActions>(
    () => ({
      openFolderDialog: () => setFolderDialogOpen(true),
      closeFolderDialog: () => setFolderDialogOpen(false),
      openFolder,
      openSample: loadSample,
      clearRecents: () => {
        writePref(RECENTS_KEY, '[]')
        setRecents([])
      },
      rescan: () => void rescan(),
      loadFile,
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
      refreshStale: () => {
        // Accept the disk version: forget the old copies; the next send reads them again.
        for (const p of stale) {
          sentMtimes.current.delete(p)
          contents.current.delete(p)
        }
        setStale(new Set())
      },

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
      apply,
      undoApply,
      dismissToast: () => setToast(null),

      updateSettings: (patch) => setSettings((s) => ({ ...s, ...patch })),
    }),
    [openFolder, loadSample, rescan, loadFile, screen, settingsReturn, toggleSet, tree, stale, send, refreshHealth, changes, apply, undoApply],
  )

  const value: AppContextValue = {
    screen,
    theme,
    project,
    recents,
    folderDialogOpen,
    lastFolder: recents[0]?.path ?? null,
    scanning,
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
