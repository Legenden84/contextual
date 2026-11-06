import { createContext, useContext } from 'react'
import type { Change, Deployment, Rule, TreeNode } from '@/data/types'
import type { ApiContent, ApiKind, ApiMessage, FileContent, GitStatus, Health, ThreadSummary } from '@/lib/api'
import type { Attachment } from '@/lib/images'

export type Screen = 'welcome' | 'workspace' | 'settings'
export type Theme = 'dark' | 'light'
export type DiffMode = 'unified' | 'split'
export type CenterView = 'chat' | 'diff'
export type SidebarTab = 'files' | 'rules' | 'scm'

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
  /** User: the content actually sent, including any context block and pictures. */
  sent?: ApiContent
  /** User: pictures attached to the message. */
  images?: { name: string; url: string }[]
  /** Note: colour of the notice. */
  tone?: 'warn' | 'error' | 'ok'
  /** Assistant: which Azure API answered (Responses keeps the conversation on Azure's side). */
  api?: ApiKind
  /** Assistant: tokens reported by Azure for this reply. */
  usage?: { inputTokens: number; outputTokens: number; cachedTokens: number }
  /** Assistant: cost of this reply in USD, at the prices when it was sent (absent if prices weren't set). */
  cost?: number
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
  tone?: 'error'
  /** Backup to restore on Undo (real folders). */
  backupId?: string
  /** Offer "Commit…" (opens Source Control) next to Undo. */
  offerCommit?: boolean
}

/** The opened project: a folder on this machine. */
export interface Project {
  name: string
  /** Absolute path on the machine the server runs on. */
  root: string
  branch: string | null
  /** The scan stopped early because the folder is very large. */
  truncated: boolean
}

export interface RecentProject {
  name: string
  path: string
  /** ISO timestamp. */
  openedAt: string
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

/** The conversation as the model has seen it, so each turn sends only what's new. */
export interface ThreadState {
  id: string
  /** Azure's id for the last reply (Responses API); null when the transcript must be sent. */
  responseId: string | null
  /** Which API answered last. */
  api: ApiKind | null
  /** Turns exactly as sent and received, in order (a stable prefix for prompt caching). */
  transcript: ApiMessage[]
  /** Version (hash) of each file the model has, by path. */
  fileVersions: Record<string, string>
  /** Files saved from this chat's replies and not yet in a pull request. */
  applied: string[]
  /** Pull requests created from this chat. */
  pullRequests: { url: string; number: number | null; branch: string }[]
}

export interface Totals {
  files: number
  rules: number
  fileTokens: number
  ruleTokens: number
  /** Estimated input tokens of the next request (everything the model reads). */
  tokens: number
  /** Of those, the part that is new: files not yet sent, pictures, the message. */
  newTokens: number
  /** Of those, the earlier conversation (re-read by the model, often from cache). */
  conversationTokens: number
  /** Share of the active deployment's context limit, 0–1+. */
  share: number
  level: 'ok' | 'warn' | 'danger'
  /** Estimated cost of the next message in USD. */
  cost: number
  /** Cost of the replies so far in this chat (USD), from Azure's reported token counts. */
  chatCost: number
  /** Tokens billed so far in this chat. */
  chatInputTokens: number
  /** Of the input tokens, how many Azure served from its prompt cache. */
  chatCachedTokens: number
  chatOutputTokens: number
  /** Replies in this chat that have usage. */
  chatReplies: number
  hint: string | null
}

export interface AppState {
  screen: Screen
  theme: Theme

  project: Project | null
  recents: RecentProject[]
  folderDialogOpen: boolean
  /** Folder of the last opened project, where the Open folder dialog starts. */
  lastFolder: string | null
  scanning: boolean

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
  /** Send every ticked file again with the next message, even if unchanged. */
  resendAll: boolean
  thread: ThreadState
  /** Saved conversations for the open folder, newest first. */
  threads: ThreadSummary[]
  messages: ChatMessage[]
  streaming: boolean
  /** Pictures waiting to be sent with the next message. */
  attachments: Attachment[]

  previewTabs: string[]
  activeTab: 'chat' | string
  centerView: CenterView
  diffMode: DiffMode
  diffFocus: string | null

  changes: Change[]
  response: ResponseInfo | null
  include: Readonly<Record<string, boolean>>
  applyOpen: boolean
  prOpen: boolean
  /** Left column tab. */
  sidebarTab: SidebarTab
  /** Bumped when git state may have changed (save, commit, rescan); Source Control refreshes on it. */
  gitTick: number
  /** Git status of the open folder; null when it isn't a repository (or not loaded yet). */
  git: GitStatus | null
  toast: ToastState | null

  settings: Settings
  totals: Totals
}

export interface AppActions {
  openFolderDialog(): void
  closeFolderDialog(): void
  /** Scans and opens a folder. Resolves to an error message, or null on success. */
  openFolder(path: string): Promise<string | null>
  clearRecents(): void
  rescan(): void
  /** Contents of a project file (cached after the first read). */
  loadFile(path: string): Promise<FileContent>
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
  setResendAll(on: boolean): void
  /** Starts a fresh conversation (the current one stays saved). */
  newChat(): void
  /** Re-opens a saved conversation. */
  openThread(id: string): void
  send(text: string): void
  /** Adds pasted/dropped/picked pictures. Resolves to an error message, or null. */
  addAttachments(files: File[]): Promise<string | null>
  removeAttachment(id: string): void
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
  openPr(): void
  closePr(): void
  /** Records a created or updated PR in the chat. */
  recordPr(pr: { url: string; number: number | null; branch: string; updatedExisting: boolean }): void
  setSidebarTab(tab: SidebarTab): void
  /** Opens a diff tab for a changed file (staged: HEAD→index, else index→working tree). */
  openGitDiff(path: string, staged: boolean): void
  /** Tell views that git state changed (after a commit, push, stage…). */
  gitChanged(): void
  closeApply(): void
  /** Writes the selected changes. Resolves to an error message, or null on success. */
  apply(backup: boolean): Promise<string | null>
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
