import { createContext, useContext } from 'react'
import type { Change, Deployment, Rule, TreeNode } from '@/data/sample'
import type { Health } from '@/lib/api'

export type Screen = 'welcome' | 'workspace' | 'settings'
export type Theme = 'dark' | 'light'
export type DiffMode = 'unified' | 'split'
export type ContextMode = 'full' | 'new'
export type CenterView = 'chat' | 'diff'

export interface ChatMessage {
  id: string
  role: 'user' | 'assistant' | 'note'
  text: string
  time: string
  /** User: what was sent with it ("12 files · 3 rules · full context"). Assistant: usage and cost. */
  meta?: string
  /** Assistant: deployment that answered. */
  model?: string
  /** Assistant: file changes parsed from the response. */
  changes?: Change[]
  /** Assistant: still receiving text. */
  streaming?: boolean
  /** User: the content actually sent, including any context block. */
  sent?: string
  /** Note: colour of the notice. */
  tone?: 'warn' | 'error'
}

export type FoundryStatus =
  | { state: 'checking' }
  | { state: 'ready'; health: Extract<Health, { configured: true }> }
  | { state: 'unconfigured'; missing: string[]; invalid: string[] }
  | { state: 'offline'; error: string }

/** The response whose changes the Changes panel and diff view show. */
export interface ResponseInfo {
  id: string
  number: number
  time: string
  model: string
}

export interface ToastState {
  title: string
  detail: string
  undoable: boolean
}

export interface Settings {
  deployments: Deployment[]
  defaultDeployment: number
  exclusions: string[]
  maxFileSizeKb: number
  respectGitignore: boolean
  skipBinary: boolean
  includeHidden: boolean
  outputPrompt: string
}

export interface Totals {
  files: number
  rules: number
  fileTokens: number
  ruleTokens: number
  tokens: number
  /** Share of the active deployment's context limit, 0–1+. */
  share: number
  level: 'ok' | 'warn' | 'danger'
  /** Estimated cost of the next message in USD. */
  cost: number
  hint: string | null
}

export interface AppState {
  screen: Screen
  theme: Theme

  tree: TreeNode[]
  selected: ReadonlySet<string>
  open: ReadonlySet<string>
  stale: ReadonlySet<string>
  filter: string

  rules: Rule[]
  rulesOn: ReadonlySet<string>
  rulePreview: string | null

  foundry: FoundryStatus
  deployment: number
  contextMode: ContextMode
  messages: ChatMessage[]
  streaming: boolean

  previewTabs: string[]
  activeTab: 'chat' | string
  centerView: CenterView
  diffMode: DiffMode
  diffFocus: string | null

  changes: Change[]
  response: ResponseInfo | null
  include: Readonly<Record<string, boolean>>
  applyOpen: boolean
  toast: ToastState | null

  settings: Settings
  totals: Totals
}

export interface AppActions {
  openProject(): void
  closeProject(): void
  openSettings(): void
  closeSettings(): void
  toggleTheme(): void

  toggleFile(path: string): void
  toggleFolder(node: TreeNode): void
  selectAll(): void
  clearSelection(): void
  toggleOpen(path: string): void
  setFilter(q: string): void
  refreshStale(): void

  toggleRule(path: string): void
  previewRule(path: string | null): void
  importRule(file: File): void

  setDeployment(i: number): void
  setContextMode(m: ContextMode): void
  send(text: string): void
  stop(): void
  refreshHealth(): void

  openPreview(path: string): void
  closeTab(path: string): void
  setActiveTab(tab: 'chat' | string): void
  openDiff(focus?: string): void
  backToChat(): void
  setDiffMode(m: DiffMode): void

  toggleInclude(path: string): void
  setIncludeAll(on: boolean): void
  openApply(): void
  closeApply(): void
  apply(): void
  undoApply(): void
  dismissToast(): void

  updateSettings(patch: Partial<Settings>): void
}

export type AppContextValue = AppState & AppActions

export const AppContext = createContext<AppContextValue | null>(null)

export function useApp(): AppContextValue {
  const ctx = useContext(AppContext)
  if (!ctx) throw new Error('useApp must be used inside <AppProvider>')
  return ctx
}
