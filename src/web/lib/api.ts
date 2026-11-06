// Browser client for the local /api routes (src/server/api.ts). The API key never reaches
// the browser; these calls only talk to the Vite server on the same origin.

export type Health =
  | {
      configured: true
      endpoint: string
      apiVersion: string
      deployment: string
      modelName: string
      maxCompletionTokens: number
      contextLimit?: number
      inputPricePer1M?: number
      outputPricePer1M?: number
    }
  | { configured: false; missing: string[]; invalid: string[] }

export type ChatRole = 'system' | 'user' | 'assistant'

export type ContentPart = { type: 'text'; text: string } | { type: 'image_url'; image_url: { url: string; detail?: 'high' } }

/** Text, or (user messages) text and pictures. */
export type ApiContent = string | ContentPart[]

export interface ApiMessage {
  role: ChatRole
  content: ApiContent
}

/** Which Azure API answered: Responses (conversation stored by Azure) or chat completions (full transcript sent). */
export type ApiKind = 'responses' | 'chat'

export type ChatEvent =
  | { type: 'delta'; text: string }
  | { type: 'usage'; inputTokens: number; outputTokens: number; cachedTokens: number }
  | { type: 'done'; finishReason: string | null; responseId: string | null; api: ApiKind; chainReset: boolean }
  | { type: 'error'; message: string }

/** One conversation turn sent to the local server. */
export interface ConversationRequest {
  /** Rules + output format, sent with every turn. */
  system: string
  /** Earlier turns exactly as sent/received; only used if Azure doesn't hold the conversation. */
  history: ApiMessage[]
  /** The new user turn. */
  input: ApiContent
  /** Azure's id for the last reply, so only the new turn needs uploading. */
  previousResponseId: string | null
}

async function errorFrom(res: Response): Promise<Error> {
  try {
    const body = (await res.json()) as { error?: string }
    return new Error(body.error ?? `Request failed with ${res.status}.`)
  } catch {
    return new Error(`Request failed with ${res.status}.`)
  }
}

export async function fetchHealth(signal?: AbortSignal): Promise<Health> {
  const res = await fetch('/api/health', { signal })
  if (!res.ok) throw await errorFrom(res)
  return (await res.json()) as Health
}

export async function testConnection(): Promise<{ latencyMs: number; deployment: string }> {
  const res = await fetch('/api/test', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' })
  if (!res.ok) throw await errorFrom(res)
  return (await res.json()) as { latencyMs: number; deployment: string }
}

/** Streams a chat completion, calling `onEvent` for each NDJSON event. */
export async function streamChat(
  request: ConversationRequest,
  onEvent: (e: ChatEvent) => void,
  signal: AbortSignal,
): Promise<void> {
  const res = await fetch('/api/chat', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
    signal,
  })
  if (!res.ok || !res.body) throw await errorFrom(res)

  const reader = res.body.pipeThrough(new TextDecoderStream()).getReader()
  let buffer = ''
  for (;;) {
    const { value, done } = await reader.read()
    if (done) break
    buffer += value
    let nl = buffer.indexOf('\n')
    while (nl >= 0) {
      const line = buffer.slice(0, nl).trim()
      buffer = buffer.slice(nl + 1)
      if (line) onEvent(JSON.parse(line) as ChatEvent)
      nl = buffer.indexOf('\n')
    }
  }
  if (buffer.trim()) onEvent(JSON.parse(buffer) as ChatEvent)
}

// ------------------------------------------------------------------ project files

export interface DirListing {
  path: string
  parent: string | null
  dirs: { name: string; path: string; project: boolean }[]
  places: { label: string; path: string }[]
  truncated: boolean
}

export interface ScanOptions {
  exclusions: string[]
  maxFileSizeKb: number
  respectGitignore: boolean
  skipBinary: boolean
  includeHidden: boolean
}

export interface ScanNode {
  name: string
  path: string
  kind: 'dir' | 'file'
  tokens?: number
  children?: ScanNode[]
  excluded?: string
  mtimeMs?: number
  image?: boolean
  condensedFrom?: number
}

export interface ScanResult {
  name: string
  root: string
  branch: string | null
  tree: ScanNode[]
  rules: { path: string; type: 'Rule' | 'Skill'; tokens: number; content: string }[]
  files: number
  truncated: boolean
}

export interface FileContent {
  path: string
  content?: string
  /** Image files: a data URL to send as a picture. */
  image?: string
  /** Content is a condensed copy of a large HTML/SVG file (original size in bytes). */
  condensedFrom?: number
  mtimeMs?: number
  error?: string
}

export interface ApplyChange {
  path: string
  status: 'A' | 'M' | 'D'
  content?: string
  expected?: string
}

export interface ApplyResult {
  created: number
  overwritten: number
  deleted: number
  backupId: string | null
}

async function postJson<T>(url: string, body: unknown): Promise<T> {
  const res = await fetch(url, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) })
  if (!res.ok) throw await errorFrom(res)
  return (await res.json()) as T
}

export async function listDirs(path: string | null, hidden: boolean): Promise<DirListing> {
  const q = new URLSearchParams({ path: path ?? '', hidden: hidden ? '1' : '0' })
  const res = await fetch(`/api/fs/dirs?${q}`)
  if (!res.ok) throw await errorFrom(res)
  return (await res.json()) as DirListing
}

export const scanProject = (root: string, options: ScanOptions) => postJson<ScanResult>('/api/project/scan', { root, options })

/** With `condense`, large HTML/SVG come back condensed (for sending); previews want the real file. */
export const readFiles = (root: string, paths: string[], condense = false) =>
  postJson<{ files: FileContent[] }>('/api/project/read', { root, paths, condense }).then((r) => r.files)

export const applyChanges = (root: string, changes: ApplyChange[], backup: boolean) =>
  postJson<ApplyResult>('/api/project/apply', { root, changes, backup })

export const undoApply = (root: string, backupId: string) =>
  postJson<{ restored: number; removed: number }>('/api/project/undo', { root, backupId })

// ------------------------------------------------------------------ saved conversations

export interface ThreadSummary {
  id: string
  title: string
  updatedAt: string
  turns: number
}

export async function listThreads(root: string): Promise<ThreadSummary[]> {
  const res = await fetch(`/api/threads?${new URLSearchParams({ root })}`)
  if (!res.ok) throw await errorFrom(res)
  return ((await res.json()) as { threads: ThreadSummary[] }).threads
}

export async function loadThread<T>(root: string, id: string): Promise<T> {
  const res = await fetch(`/api/threads/${encodeURIComponent(id)}?${new URLSearchParams({ root })}`)
  if (!res.ok) throw await errorFrom(res)
  return (await res.json()) as T
}

export const saveThread = (root: string, id: string, summary: { title: string; turns: number }, thread: unknown) =>
  postJson<{ ok: true }>('/api/threads/save', { root, id, summary, thread })

export const deleteThread = (root: string, id: string) => postJson<{ ok: true }>('/api/threads/delete', { root, id })

// ------------------------------------------------------------------ git (Source Control) and pull requests

/** VS Code-style letters: M modified, A added, D deleted, R renamed, U untracked, C conflict. */
export type ChangeLetter = 'M' | 'A' | 'D' | 'R' | 'U' | 'C'

export interface StatusEntry {
  path: string
  from?: string
  letter: ChangeLetter
}

export interface GitStatus {
  branch: string | null
  upstream: string | null
  ahead: number
  behind: number
  staged: StatusEntry[]
  changes: StatusEntry[]
  conflicts: StatusEntry[]
}

export interface FileDiff {
  path: string
  old: string
  new: string
  skipped?: string
}

export interface BranchInfo {
  /** Local: "feature/x"; remote: "origin/feature/x". */
  name: string
  upstream: string | null
  sha: string
  when: string
  subject: string
}

export interface BranchList {
  current: string | null
  local: BranchInfo[]
  remote: BranchInfo[]
}

export interface Comparison {
  /** Source branch (compare). */
  branch: string | null
  /** The checked-out branch. */
  current: string | null
  defaultBranch: string | null
  base: string
  remote: string | null
  github: boolean
  problem: string | null
  commits: { sha: string; subject: string; author: string; when: string }[]
  files: (FileDiff & { letter: ChangeLetter })[]
  uncommitted: number
  unpushed: number
  existingPr: { url: string; number: number } | null
}

export interface PrResult {
  url: string
  number: number | null
  branch: string
  updatedExisting: boolean
}

export const gitStatus = (root: string) => postJson<GitStatus>('/api/git/status', { root })
export const gitStage = (root: string, paths: string[]) => postJson<GitStatus>('/api/git/stage', { root, paths })
export const gitUnstage = (root: string, paths: string[]) => postJson<GitStatus>('/api/git/unstage', { root, paths })
export const gitDiff = (root: string, path: string, staged: boolean) => postJson<FileDiff>('/api/git/diff', { root, path, staged })
export const gitCommit = (root: string, message: string, all: boolean) =>
  postJson<{ commit: string; files: number }>('/api/git/commit', { root, message, all })
export const gitPush = (root: string) => postJson<{ branch: string; upstream: string }>('/api/git/push', { root })
export const draftCommit = (root: string) => postJson<{ message: string }>('/api/git/draft-commit', { root })

export const listBranches = (root: string, fetch = false) => postJson<BranchList>('/api/git/branches', { root, fetch })
export const switchBranch = (root: string, name: string) => postJson<GitStatus>('/api/git/switch', { root, name })
export const createBranch = (root: string, name: string, from?: string) =>
  postJson<GitStatus>('/api/git/create-branch', { root, name, from })

export const comparePr = (root: string, base?: string, head?: string) => postJson<Comparison>('/api/pr/compare', { root, base, head })
export const draftPr = (root: string, base: string, notes: string, head?: string) =>
  postJson<{ title: string; body: string }>('/api/pr/draft', { root, base, notes, head })
export const createPr = (
  root: string,
  req: { title: string; body: string; base: string; head?: string; draft: boolean; newBranch?: string },
) => postJson<PrResult>('/api/pr/create', { root, ...req })

/** Rebuilds a whole file when the reply's edit blocks couldn't be placed (internal fallback). */
export const repairEdits = (path: string, original: string, edits: string) =>
  postJson<{ content: string; inputTokens: number; outputTokens: number }>('/api/edit/repair', { path, original, edits })
