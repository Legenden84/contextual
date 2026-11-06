import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { defaultExclusions, EST_OUTPUT_TOKENS, outputPrompt, PLACEHOLDER_DEPLOYMENT, USER_MESSAGE_TOKENS } from '@/data/defaults'
import type { Change, Rule, TreeNode } from '@/data/types'
import * as api from '@/lib/api'
import { parseChanges } from '@/lib/changes'
import { estimateTokens, fmtK } from '@/lib/format'
import { parseMarkdown } from '@/lib/markdown'
import { toAttachment, type Attachment } from '@/lib/images'
import { gitTabId } from '@/lib/git-tabs'
import { hashText } from '@/lib/hash'
import { buildTurn, contentImages, contentText, systemPrompt, type ContextFile } from '@/lib/prompt'
import { biggestFolder, checkState, findNode, nodeStats, pickableFiles, selectableFiles } from '@/lib/tree'
import {
  AppContext,
  type AppActions,
  type AppContextValue,
  type ChatMessage,
  type DiffMode,
  type FoundryStatus,
  type Project,
  type RecentProject,
  type ResponseInfo,
  type Screen,
  type Settings,
  type SidebarTab,
  type Theme,
  type ThreadState,
  type ToastState,
  type Totals,
} from './app-context'

const DIFF_MODE_KEY = 'contextual.diffMode'
const THEME_KEY = 'contextual.theme'
const RECENTS_KEY = 'contextual.recentProjects'
const SETTINGS_KEY = 'contextual.settings'
/** The open folder, per browser tab, so a page reload (e.g. Vite reloading after a save) reopens it. */
const OPEN_PROJECT_KEY = 'contextual.openProject'

function rememberOpen(root: string | null) {
  try {
    if (root) sessionStorage.setItem(OPEN_PROJECT_KEY, root)
    else sessionStorage.removeItem(OPEN_PROJECT_KEY)
  } catch {
    // Without session storage a reload just returns to the Welcome screen.
  }
}
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
  deployments: [PLACEHOLDER_DEPLOYMENT],
  defaultDeployment: 0,
  exclusions: defaultExclusions,
  maxFileSizeKb: 256,
  respectGitignore: true,
  skipBinary: true,
  includeHidden: false,
  outputPrompt,
}

/** Settings saved in this browser, merged over the defaults (unknown or mistyped fields are ignored). */
function readSettings(): Settings {
  try {
    const saved = JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? '{}') as Partial<Settings>
    const s = { ...initialSettings }
    if (Array.isArray(saved.exclusions) && saved.exclusions.every((x) => typeof x === 'string')) s.exclusions = saved.exclusions
    if (typeof saved.maxFileSizeKb === 'number' && saved.maxFileSizeKb > 0) s.maxFileSizeKb = saved.maxFileSizeKb
    for (const k of ['respectGitignore', 'skipBinary', 'includeHidden'] as const) if (typeof saved[k] === 'boolean') s[k] = saved[k]
    if (typeof saved.outputPrompt === 'string' && saved.outputPrompt.trim()) s.outputPrompt = saved.outputPrompt
    return s
  } catch {
    return initialSettings
  }
}

let nextId = 0
const newId = (prefix: string) => `${prefix}${Date.now().toString(36)}${(nextId++).toString(36)}`

const freshThread = (): ThreadState => ({
  id: newId('t'),
  responseId: null,
  api: null,
  transcript: [],
  fileVersions: {},
  applied: [],
  pullRequests: [],
})

/** What is saved per conversation (in ~/.cache/contextual/threads via the local server). */
interface SavedThread extends ThreadState {
  version: 1
  messages: ChatMessage[]
}

/** Vision tokens assumed per picture already in the conversation. */
const PICTURE_TOKENS = 765

function fileTokenMap(nodes: TreeNode[], out = new Map<string, number>()): Map<string, number> {
  for (const n of nodes) {
    if (n.kind === 'file') out.set(n.path, n.tokens ?? 0)
    if (n.children) fileTokenMap(n.children, out)
  }
  return out
}

function lastChanges(messages: ChatMessage[]): Change[] {
  for (let i = messages.length - 1; i >= 0; i--) if (messages[i].changes?.length) return messages[i].changes!
  return []
}

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
  const [resendAll, setResendAll] = useState(false)
  const [thread, setThread] = useState<ThreadState>(freshThread)
  const [threads, setThreads] = useState<api.ThreadSummary[]>([])
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [attachments, setAttachments] = useState<Attachment[]>([])
  /** Files sent as condensed references: replies can't change them. */
  const referenceOnly = useRef(new Set<string>())
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
  const [prOpen, setPrOpen] = useState(false)
  const [sidebarTab, setSidebarTab] = useState<SidebarTab>('files')
  const [gitTick, setGitTick] = useState(0)
  const [git, setGit] = useState<api.GitStatus | null>(null)
  const [toast, setToast] = useState<ToastState | null>(null)

  const [settings, setSettings] = useState<Settings>(readSettings)

  useEffect(() => {
    const { deployments: _fromServer, defaultDeployment: _default, ...saved } = settings
    void _fromServer
    void _default
    writePref(SETTINGS_KEY, JSON.stringify(saved))
  }, [settings])

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
        setSettings((s) => {
          // .env.local wins for limit and prices; otherwise keep what was typed in Settings.
          const saved = s.deployments.find((d) => d.name === h.deployment)
          return {
            ...s,
            defaultDeployment: 0,
            deployments: [
              {
                name: h.deployment,
                limit: h.contextLimit ?? saved?.limit ?? FALLBACK_CONTEXT_LIMIT,
                inPrice: h.inputPricePer1M ?? saved?.inPrice ?? 0,
                outPrice: h.outputPricePer1M ?? saved?.outPrice ?? 0,
              },
            ],
          }
        })
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
    referenceOnly.current.clear()
    setAttachments([])
    setFilter('')
    setRulePreview(null)
    setActiveTab('chat')
    setCenterView('chat')
    setDiffFocus(null)
    setApplyOpen(false)
    setToast(null)
  }, [])

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

  /** Shows a saved conversation and re-ticks the files the model has from it. */
  const applySaved = useCallback((saved: SavedThread, pickable?: Set<string>) => {
    setThread({
      id: saved.id,
      responseId: saved.responseId,
      api: saved.api,
      transcript: saved.transcript,
      fileVersions: saved.fileVersions,
      applied: saved.applied ?? [],
      pullRequests: saved.pullRequests ?? [],
    })
    setMessages(saved.messages.map((m) => ({ ...m, streaming: false })))
    setInclude(includeFor(lastChanges(saved.messages)))
    if (pickable) setSelected(new Set(Object.keys(saved.fileVersions).filter((p) => pickable.has(p))))
    setCenterView('chat')
    setActiveTab('chat')
  }, [])

  const refreshThreads = useCallback(async (root: string) => {
    try {
      const list = await api.listThreads(root)
      setThreads(list)
      return list
    } catch {
      return []
    }
  }, [])

  const openFolder = useCallback(
    async (path: string): Promise<string | null> => {
      try {
        setScanning(true)
        const r = await api.scanProject(path, scanOptions)
        lastScanKey.current = JSON.stringify(scanOptions)
        resetWorkspace()
        setProject({ name: r.name, root: r.root, branch: r.branch, truncated: r.truncated })
        setRawTree(r.tree)
        setSelected(new Set())
        setOpen(new Set())
        setStale(new Set())
        setRules(r.rules)
        setRulesOn(new Set(r.rules.filter((x) => x.type === 'Rule').map((x) => x.path)))
        setMessages([])
        setThread(freshThread())
        setThreads([])
        setPreviewTabs([])
        setInclude({})
        setFolderDialogOpen(false)
        setScreen('workspace')
        // Pick up the latest conversation for this folder, if any.
        void refreshThreads(r.root).then(async (list) => {
          if (!list.length) return
          try {
            applySaved(await api.loadThread<SavedThread>(r.root, list[0].id), pickableFiles(r.tree))
          } catch {
            // A missing or unreadable conversation just means starting fresh.
          }
        })
        rememberOpen(r.root)
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
    [scanOptions, resetWorkspace, refreshThreads, applySaved],
  )

  /** Re-reads the open folder: new/removed files, exclusions, and files changed since they were sent. */
  const rescan = useCallback(async () => {
    if (!project) return
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
      const pickable = pickableFiles(r.tree)
      setSelected((s) => new Set([...s].filter((p) => pickable.has(p))))
      setStale(changed)
      setGitTick((t) => t + 1)
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
    if (!project || scanKey === lastScanKey.current) return
    const t = window.setTimeout(() => void rescan(), 400)
    return () => window.clearTimeout(t)
  }, [scanKey, project, rescan])

  // Coming back to the window: pick up files changed in the editor.
  const lastFocusScan = useRef(0)
  useEffect(() => {
    if (!project) return
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

  // #open=/path/to/project opens that folder directly (bookmarkable). Otherwise, after a reload,
  // reopen the folder this tab had open (its latest chat is restored with it).
  const openedOnLoad = useRef(false)
  useEffect(() => {
    if (openedOnLoad.current) return
    openedOnLoad.current = true
    const m = /^#open=(.+)$/.exec(window.location.hash)
    let path: string | null = m ? decodeURIComponent(m[1]) : null
    if (!path) {
      try {
        path = sessionStorage.getItem(OPEN_PROJECT_KEY)
      } catch {
        path = null
      }
    }
    if (!path) return
    void openFolder(path).then((err) => {
      if (!err) return
      rememberOpen(null)
      setToast({ title: 'Could not open the folder', detail: err, undoable: false, tone: 'error' })
    })
  }, [openFolder])

  // ---------------------------------------------------------------- derived state

  const tree = useMemo(
    () => withStale(rawTree, stale),
    [rawTree, stale],
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
    // The model re-reads the whole conversation each turn; only files it doesn't have are new.
    const conversationTokens = thread.transcript.reduce(
      (a, m) => a + estimateTokens(contentText(m.content)) + contentImages(m.content) * PICTURE_TOKENS,
      0,
    )
    const sizes = fileTokenMap(tree)
    let newFileTokens = 0
    for (const p of selected) if (resendAll || !thread.fileVersions[p] || stale.has(p)) newFileTokens += sizes.get(p) ?? 0
    const pictures = attachments.reduce((a, x) => a + x.tokens, 0)
    const newTokens = newFileTokens + pictures + USER_MESSAGE_TOKENS
    const raw = ruleTokens + estimateTokens(settings.outputPrompt) + conversationTokens + newTokens
    const tokens = Math.round(raw / 100) * 100
    const dep = settings.deployments[deployment] ?? settings.deployments[0]
    const share = tokens / dep.limit
    const level = share >= 0.95 ? 'danger' : share >= 0.75 ? 'warn' : 'ok'
    const cost = (tokens / 1e6) * dep.inPrice + (EST_OUTPUT_TOKENS / 1e6) * dep.outPrice
    const big = level === 'ok' ? null : biggestFolder(tree, selected)
    // What this chat has cost so far, from the token counts Azure reported for each reply.
    let chatCost = 0
    let chatInputTokens = 0
    let chatOutputTokens = 0
    let chatCachedTokens = 0
    let chatReplies = 0
    for (const m of messages) {
      if (!m.usage) continue
      chatReplies++
      chatInputTokens += m.usage.inputTokens
      chatCachedTokens += m.usage.cachedTokens
      chatOutputTokens += m.usage.outputTokens
      chatCost += m.cost ?? 0
    }
    return {
      chatCost,
      chatInputTokens,
      chatCachedTokens,
      chatOutputTokens,
      chatReplies,
      files,
      rules: onRules.length,
      fileTokens,
      ruleTokens,
      tokens,
      newTokens: Math.round(newTokens / 100) * 100,
      conversationTokens,
      share,
      level,
      cost,
      hint: big ? `${big.name}: ${fmtK(big.tokens)}` : null,
    }
  }, [tree, selected, rules, rulesOn, thread, resendAll, stale, attachments, messages, settings.deployments, settings.outputPrompt, deployment])

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
      const forceAll = resendAll
      const base = thread

      void (async () => {
        // Read the selected files now, so the model sees what is on disk at this moment.
        const paths = [...selected].sort()
        let read: api.FileContent[]
        try {
          read = await api.readFiles(project.root, paths, true)
        } catch (err) {
          abortRef.current = null
          addNote(`Not sent: could not read the selected files. ${errorText(err, '')}`, 'error')
          return
        }
        for (const f of read) {
          if (f.mtimeMs != null) sentMtimes.current.set(f.path, f.mtimeMs)
          if (f.condensedFrom) referenceOnly.current.add(f.path)
          else referenceOnly.current.delete(f.path)
          // Keep only real contents as diff originals; condensed copies are references.
          if (f.content != null && !f.condensedFrom) contents.current.set(f.path, f.content)
        }
        setStale((s) => new Set([...s].filter((p) => !paths.includes(p))))

        // Only files the model doesn't have yet (or that changed since) are attached.
        const versions = { ...base.fileVersions }
        const changed: ContextFile[] = []
        const unchanged: string[] = []
        for (const f of read) {
          const version = hashText(f.image ?? f.content ?? `unavailable:${f.error ?? ''}`)
          if (!forceAll && versions[f.path] === version) unchanged.push(f.path)
          else {
            changed.push({ path: f.path, content: f.content, image: f.image, condensed: !!f.condensedFrom })
            versions[f.path] = version
          }
        }
        const removed = Object.keys(versions).filter((p) => !selected.has(p))
        for (const p of removed) delete versions[p]

        const onRules = rules.filter((r) => rulesOn.has(r.path))
        const images = attachments.map((a) => ({ name: a.name, url: a.url }))
        const input = buildTurn({ text, files: changed, unchanged, removed, attachments: images })
        setAttachments([])
        setResendAll(false)

        const time = now()
        const aiId = newId('a')
        const fileNote = paths.length
          ? `${changed.length} of ${paths.length} ${paths.length === 1 ? 'file' : 'files'} sent${unchanged.length ? ` (${unchanged.length} unchanged)` : ''}`
          : 'no files'
        setMessages((ms) => [
          ...ms,
          { id: newId('u'), role: 'user', text, sent: input, images, time, meta: `${fileNote} · ${onRules.length} rules` },
          { id: aiId, role: 'assistant', text: '', time, model: dep.name, streaming: true },
        ])

        // Collected from the stream; an object so the callbacks don't reassign locals.
        const out = {
          reply: '',
          usage: null as { inputTokens: number; outputTokens: number; cachedTokens: number } | null,
          done: null as Extract<api.ChatEvent, { type: 'done' }> | null,
          failure: null as string | null,
        }
        try {
          await api.streamChat(
            {
              system: systemPrompt(onRules, settings.outputPrompt),
              history: base.transcript,
              input,
              previousResponseId: base.responseId,
            },
            (e) => {
              if (e.type === 'delta') {
                out.reply += e.text
                const reply = out.reply
                updateMessage(aiId, (m) => ({ ...m, text: reply }))
              } else if (e.type === 'usage') out.usage = e
              else if (e.type === 'done') out.done = e
              else if (e.type === 'error') out.failure = e.message
            },
            ctrl.signal,
          )
        } catch (err) {
          out.failure = ctrl.signal.aborted ? 'Stopped.' : errorText(err, 'Request failed.')
        }
        abortRef.current = null

        const { reply, usage: u, done, failure } = out
        // Remember the turn only if it completed; otherwise the next message resends what this one tried to.
        if (reply && done && !failure) {
          setThread((t) =>
            t.id !== base.id
              ? t
              : {
                  ...t,
                  responseId: done.responseId,
                  api: done.api,
                  transcript: [...t.transcript, { role: 'user', content: input }, { role: 'assistant', content: reply }],
                  fileVersions: versions,
                },
          )
        }

        // Originals for every file the reply touches, so diffs compare against disk.
        const touched = parseMarkdown(reply)
          .map((b) => (b.kind === 'code' ? b.path : b.kind === 'delete' ? b.path : undefined))
          .filter((p): p is string => !!p && !!findNode(tree, p))
        const missing = touched.filter((p) => !contents.current.has(p))
        if (missing.length) {
          try {
            for (const f of await api.readFiles(project.root, missing)) if (f.content != null) contents.current.set(f.path, f.content)
          } catch {
            // Diffs for those files will show the whole file as added.
          }
        }
        const truncated = done?.finishReason === 'length'
        const proposed = truncated
          ? []
          : parseChanges(reply, {
              exists: (p) => !!findNode(tree, p),
              original: (p) => contents.current.get(p),
              changedOnDisk: (p) => stale.has(p),
              readOnly: (p) => referenceOnly.current.has(p),
            })

        const priced = dep.inPrice > 0 || dep.outPrice > 0
        const cost = u && priced ? (u.inputTokens / 1e6) * dep.inPrice + (u.outputTokens / 1e6) * dep.outPrice : undefined
        const meta = u
          ? `${fmtK(u.inputTokens)} in${u.cachedTokens ? ` (${fmtK(u.cachedTokens)} cached)` : ''} / ${fmtK(u.outputTokens)} out` +
            (cost != null ? ` · $${cost.toFixed(3)}` : '')
          : undefined

        setMessages((ms) => {
          const next = ms.flatMap<ChatMessage>((m) => {
            if (m.id !== aiId) return [m]
            if (!reply) return []
            return [{ ...m, streaming: false, meta, changes: proposed, api: done?.api, usage: u ?? undefined, cost }]
          })
          const note = (t: string, tone: 'warn' | 'error') => next.push({ id: newId('n'), role: 'note', text: t, time: now(), tone })
          if (failure) note(failure, failure === 'Stopped.' ? 'warn' : 'error')
          if (done?.chainReset && done.api === 'responses')
            note('Azure no longer had this conversation stored, so the full conversation was sent once and a new chain started.', 'warn')
          if (done?.api === 'chat' && base.api === 'responses')
            note('The Responses API is not available for this deployment; the full conversation is now sent with each message.', 'warn')
          if (truncated)
            note(
              'The reply hit the completion token limit and was cut off, so its file changes were not offered for saving. Raise AZURE_OPENAI_MAX_COMPLETION_TOKENS or ask for fewer files.',
              'warn',
            )
          return next
        })
        if (proposed.length) setInclude(includeFor(proposed))
      })()
    },
    [project, settings, deployment, resendAll, thread, attachments, selected, rules, rulesOn, stale, tree, updateMessage, addNote],
  )

  // Git status of the open folder, refreshed whenever git may have changed.
  useEffect(() => {
    if (!project) return
    let live = true
    api
      .gitStatus(project.root)
      .then((s) => live && setGit(s))
      .catch(() => live && setGit(null))
    return () => {
      live = false
    }
  }, [project, gitTick])

  // Save the conversation after each completed turn.
  useEffect(() => {
    if (!project || streaming || !thread.transcript.length) return
    const root = project.root
    const t = window.setTimeout(() => {
      const title = messages.find((m) => m.role === 'user')?.text.slice(0, 80) ?? 'Conversation'
      const saved: SavedThread = { version: 1, ...thread, messages }
      void api
        .saveThread(root, thread.id, { title, turns: thread.transcript.length / 2 }, saved)
        .then(() => refreshThreads(root))
        .catch(() => setToast({ title: 'Could not save the conversation', detail: 'It stays open, but won’t be restored after a reload.', undoable: false, tone: 'error' }))
    }, 500)
    return () => window.clearTimeout(t)
  }, [project, thread, messages, streaming, refreshThreads])

  // ---------------------------------------------------------------- apply

  const apply = useCallback(
    async (backup: boolean): Promise<string | null> => {
      const picked = changes.filter((c) => include[c.path])
      if (!picked.length || !project) return null
      const count = (s: string) => picked.filter((c) => c.status === s).length
      const parts = [count('D') && `${count('D')} deleted`, count('A') && `${count('A')} created`, count('M') && `${count('M')} overwritten`].filter(Boolean)
      const title = `${picked.length} ${picked.length === 1 ? 'file' : 'files'} saved`

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
        // The model wrote these versions itself, so they don't need sending back next turn.
        setThread((t) => {
          const fileVersions = { ...t.fileVersions }
          for (const c of picked) {
            if (c.status === 'D') delete fileVersions[c.path]
            else fileVersions[c.path] = hashText(c.new)
          }
          // Remember what this chat saved, for "Create pull request".
          const applied = [...new Set([...t.applied, ...picked.map((c) => c.path)])]
          return { ...t, fileVersions, applied }
        })
        setApplyOpen(false)
        setToast({
          title,
          detail: `${parts.join(', ')} · not committed${project.branch ? ` on ${project.branch}` : ''}`,
          undoable: !!r.backupId,
          backupId: r.backupId ?? undefined,
          offerCommit: true,
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
    if (!project || !toast?.backupId) return
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
      clearRecents: () => {
        writePref(RECENTS_KEY, '[]')
        setRecents([])
      },
      rescan: () => void rescan(),
      loadFile,
      closeProject: () => {
        rememberOpen(null)
        setScreen('welcome')
      },
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
      setResendAll,
      newChat: () => {
        if (abortRef.current) return
        referenceOnly.current.clear()
        setThread(freshThread())
        setMessages([])
        setInclude({})
        setAttachments([])
        setCenterView('chat')
        setActiveTab('chat')
      },
      openThread: (id) => {
        if (abortRef.current || !project) return
        const root = project.root
        void api
          .loadThread<SavedThread>(root, id)
          .then((saved) => applySaved(saved, pickableFiles(rawTree)))
          .catch((err: unknown) => setToast({ title: 'Could not open the conversation', detail: errorText(err, ''), undoable: false, tone: 'error' }))
      },
      send,
      addAttachments: async (files) => {
        const errors: string[] = []
        for (const f of files) {
          try {
            const a = await toAttachment(f, newId('img'))
            setAttachments((list) => [...list, a])
          } catch (err) {
            errors.push(errorText(err, `${f.name}: could not read the picture.`))
          }
        }
        return errors.length ? errors.join(' ') : null
      },
      removeAttachment: (id) => setAttachments((list) => list.filter((a) => a.id !== id)),
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
      openPr: () => {
        setToast(null)
        setPrOpen(true)
      },
      closePr: () => setPrOpen(false),
      recordPr: (pr) => {
        setThread((t) => ({
          ...t,
          pullRequests: [...t.pullRequests, { url: pr.url, number: pr.number, branch: pr.branch }],
        }))
        const label = pr.number ? `#${pr.number}` : ''
        setMessages((ms) => [
          ...ms,
          {
            id: newId('n'),
            role: 'note',
            tone: 'ok',
            time: now(),
            text: pr.updatedExisting
              ? `Pushed ${pr.branch}; pull request ${label} is up to date: ${pr.url}`
              : `Pull request ${label} opened from ${pr.branch}: ${pr.url}`,
          },
        ])
        setPrOpen(false)
        void rescan()
      },
      setSidebarTab,
      openGitDiff: (path, staged) => {
        const tab = gitTabId(path, staged)
        setPreviewTabs((tabs) => (tabs.includes(tab) ? tabs : [...tabs, tab]))
        setActiveTab(tab)
        setCenterView('chat')
      },
      gitChanged: () => setGitTick((t) => t + 1),
      closeApply: () => setApplyOpen(false),
      apply,
      undoApply,
      dismissToast: () => setToast(null),

      updateSettings: (patch) => setSettings((s) => ({ ...s, ...patch })),
    }),
    [rescan, openFolder, loadFile, screen, settingsReturn, toggleSet, tree, stale, send, refreshHealth, changes, apply, undoApply, project, rawTree, applySaved],
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
    resendAll,
    thread,
    threads,
    messages,
    streaming,
    attachments,
    previewTabs,
    activeTab,
    centerView,
    diffMode,
    diffFocus,
    changes,
    response,
    include,
    applyOpen,
    prOpen,
    sidebarTab,
    gitTick,
    git,
    toast,
    settings,
    totals,
    ...actions,
  }

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>
}
