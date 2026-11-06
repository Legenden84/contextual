// HTTP routes under /api, mounted as Connect middleware on the local Vite server.
//
//   GET  /api/health      Configuration status (never the API key).
//   POST /api/test        Round-trip to the deployment, returns latency.
//   POST /api/chat        { system, history, input, previousResponseId } → NDJSON stream of ChatEvent.
//   GET  /api/fs/dirs     ?path=&hidden=1 → sub-folders, for the Open folder dialog.
//   POST /api/project/scan   { root, options } → explorer tree, rules, branch.
//   POST /api/project/read   { root, paths } → file contents.
//   POST /api/project/apply  { root, changes, backup } → writes the selected changes.
//   POST /api/project/undo   { root, backupId } → restores from that backup.
//   GET  /api/threads        ?root= → saved conversations for a project, newest first.
//   GET  /api/threads/<id>   ?root= → one saved conversation.
//   POST /api/threads/save   { root, id, summary, thread } → saves a conversation.
//   POST /api/threads/delete { root, id } → deletes one.
//   POST /api/git/info       { root, paths } → branch, default branch, GitHub access, file states, open PR.
//   POST /api/pr/draft       { root, paths, notes } → model-written { title, body }.
//   POST /api/pr/create      { root, paths, title, body, base, draft, newBranch? } → commits, pushes, opens the PR.

import type { IncomingMessage, ServerResponse } from 'node:http'
import { readFoundryConfig, type ConfigResult, type FoundryConfig } from './config.ts'
import {
  createFoundryClients,
  describeError,
  draftPullRequest,
  streamConversation,
  testConnection,
  type ChatMessage,
  type ContentPart,
  type ConversationRequest,
  type FoundryClients,
} from './foundry.ts'
import { listDirs } from './fs/browse.ts'
import { applyChanges, readFiles, undoApply, type ApplyChange } from './fs/files.ts'
import { PathError } from './fs/paths.ts'
import { scanProject, type ScanOptions } from './fs/scan.ts'
import { createPullRequest, diffFor, gitInfo } from './git.ts'
import { deleteThread, listThreads, loadThread, saveThread } from './threads.ts'

type Env = Record<string, string | undefined>
type Next = (err?: unknown) => void

/** Chat requests can carry several pictures as base64 (up to 20 MB each at Azure's limit). */
const MAX_BODY_BYTES = 64 * 1024 * 1024
const ROLES = new Set(['system', 'user', 'assistant'])

class HttpError extends Error {
  status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

function sendJson(res: ServerResponse, status: number, body: unknown) {
  res.statusCode = status
  res.setHeader('Content-Type', 'application/json; charset=utf-8')
  res.setHeader('Cache-Control', 'no-store')
  res.end(JSON.stringify(body))
}

/**
 * Only this app's own pages may call the API. Browsers send Origin on cross-site requests,
 * and requiring a JSON body forces a CORS preflight, so another site can't spend tokens.
 */
function assertSameOrigin(req: IncomingMessage) {
  const origin = req.headers.origin
  if (origin && new URL(origin).host !== req.headers.host) throw new HttpError(403, 'Cross-origin requests are not allowed.')
}

async function readJson(req: IncomingMessage): Promise<unknown> {
  if (!req.headers['content-type']?.startsWith('application/json'))
    throw new HttpError(415, 'Expected Content-Type: application/json.')
  const chunks: Buffer[] = []
  let size = 0
  for await (const chunk of req) {
    size += (chunk as Buffer).length
    if (size > MAX_BODY_BYTES) throw new HttpError(413, 'Request body is too large.')
    chunks.push(chunk as Buffer)
  }
  try {
    return JSON.parse(Buffer.concat(chunks).toString('utf8'))
  } catch {
    throw new HttpError(400, 'Request body is not valid JSON.')
  }
}

function parseMessages(list: unknown, name: string): ChatMessage[] {
  if (!Array.isArray(list)) throw new HttpError(400, `\`${name}\` must be an array.`)
  return list.map((m, i): ChatMessage => {
    const { role, content } = (m ?? {}) as { role?: unknown; content?: unknown }
    if (typeof role !== 'string' || !ROLES.has(role) || role === 'system')
      throw new HttpError(400, `${name}[${i}] needs a role (user or assistant).`)
    if (typeof content === 'string') return { role: role as 'user', content }
    if (role === 'user' && Array.isArray(content) && content.length)
      return { role: 'user', content: content.map((p, j) => contentPart(p, `${name}[${i}].content[${j}]`)) }
    throw new HttpError(400, `${name}[${i}] needs string content (or text and image parts for user messages).`)
  })
}

function parseConversation(body: unknown): ConversationRequest {
  if (!isObj(body)) throw new HttpError(400, 'Request body must be a JSON object.')
  if (typeof body.system !== 'string') throw new HttpError(400, '`system` must be a string.')
  const input = body.input
  const parsedInput =
    typeof input === 'string' && input
      ? input
      : Array.isArray(input) && input.length
        ? input.map((p, j) => contentPart(p, `input[${j}]`))
        : null
  if (!parsedInput) throw new HttpError(400, '`input` must be a non-empty string or array of parts.')
  const prev = body.previousResponseId
  if (prev != null && (typeof prev !== 'string' || !/^[\w-]{1,200}$/.test(prev)))
    throw new HttpError(400, '`previousResponseId` must be a response id or null.')
  return { system: body.system, history: parseMessages(body.history ?? [], 'history'), input: parsedInput, previousResponseId: prev ?? null }
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null

function str(body: Record<string, unknown>, key: string): string {
  const v = body[key]
  if (typeof v !== 'string' || !v.trim()) throw new HttpError(400, `\`${key}\` must be a non-empty string.`)
  return v
}

function strList(body: Record<string, unknown>, key: string, max: number): string[] {
  const v = body[key]
  if (!Array.isArray(v) || v.some((x) => typeof x !== 'string')) throw new HttpError(400, `\`${key}\` must be an array of strings.`)
  if (v.length > max) throw new HttpError(400, `\`${key}\` has more than ${max} entries.`)
  return v as string[]
}

function scanOptions(v: unknown): ScanOptions {
  const o = isObj(v) ? v : {}
  return {
    exclusions: Array.isArray(o.exclusions) ? o.exclusions.filter((x): x is string => typeof x === 'string') : [],
    maxFileSizeKb: typeof o.maxFileSizeKb === 'number' && o.maxFileSizeKb > 0 ? o.maxFileSizeKb : 256,
    respectGitignore: o.respectGitignore !== false,
    skipBinary: o.skipBinary !== false,
    includeHidden: o.includeHidden === true,
  }
}

function applyList(v: unknown): ApplyChange[] {
  if (!Array.isArray(v) || !v.length) throw new HttpError(400, '`changes` must be a non-empty array.')
  return v.map((c, i) => {
    if (!isObj(c) || typeof c.path !== 'string' || !['A', 'M', 'D'].includes(c.status as string))
      throw new HttpError(400, `changes[${i}] needs a path and a status (A, M, D).`)
    return {
      path: c.path,
      status: c.status as ApplyChange['status'],
      content: typeof c.content === 'string' ? c.content : undefined,
      expected: typeof c.expected === 'string' ? c.expected : undefined,
    }
  })
}

async function readObject(req: IncomingMessage): Promise<Record<string, unknown>> {
  const body = await readJson(req)
  if (!isObj(body)) throw new HttpError(400, 'Request body must be a JSON object.')
  return body
}

function contentPart(p: unknown, where: string): ContentPart {
  if (isObj(p) && p.type === 'text' && typeof p.text === 'string') return { type: 'text', text: p.text }
  if (isObj(p) && p.type === 'image_url' && isObj(p.image_url) && typeof p.image_url.url === 'string') {
    const url = p.image_url.url
    if (!/^data:image\/(png|jpeg|gif|webp);base64,/.test(url)) throw new HttpError(400, `${where}: images must be PNG, JPEG, GIF or WebP data URLs.`)
    return { type: 'image_url', image_url: { url, detail: 'high' } }
  }
  throw new HttpError(400, `${where} must be a text or image_url part.`)
}

function publicConfig(result: ConfigResult) {
  if (!result.ok) return { configured: false, missing: result.missing, invalid: result.invalid }
  const { apiKey: _secret, ...rest } = result.config
  void _secret
  return { configured: true, ...rest }
}

/** Builds the /api middleware. `getEnv` is read per request so .env.local edits apply after a restart. */
export function createApi(getEnv: () => Env) {
  let cached: { key: string; clients: FoundryClients } | null = null

  const clientsFor = (cfg: FoundryConfig) => {
    const key = JSON.stringify([cfg.endpoint, cfg.apiKey, cfg.apiVersion, cfg.deployment])
    if (cached?.key !== key) cached = { key, clients: createFoundryClients(cfg) }
    return cached.clients
  }

  const requireConfig = (): FoundryConfig => {
    const result = readFoundryConfig(getEnv())
    if (result.ok) return result.config
    const parts = [
      result.missing.length && `missing ${result.missing.join(', ')}`,
      result.invalid.length && `invalid ${result.invalid.join(', ')}`,
    ].filter(Boolean)
    throw new HttpError(503, `Foundry is not configured: ${parts.join('; ')}. See .env.example.`)
  }

  return async function api(req: IncomingMessage, res: ServerResponse, next: Next) {
    const url = new URL(req.url ?? '/', 'http://localhost')
    if (!url.pathname.startsWith('/api/')) return next()

    // Abort the upstream request if the browser goes away.
    const abort = new AbortController()
    res.on('close', () => {
      if (!res.writableFinished) abort.abort()
    })

    try {
      assertSameOrigin(req)
      const route = `${req.method} ${url.pathname}`

      if (route === 'GET /api/health') return sendJson(res, 200, publicConfig(readFoundryConfig(getEnv())))

      if (route === 'POST /api/test') {
        const cfg = requireConfig()
        const ms = await testConnection(clientsFor(cfg).chat, cfg, abort.signal)
        return sendJson(res, 200, { ok: true, latencyMs: ms, deployment: cfg.deployment })
      }

      if (route === 'POST /api/chat') {
        const cfg = requireConfig()
        const conversation = parseConversation(await readJson(req))
        const stream = streamConversation(clientsFor(cfg), cfg, conversation, abort.signal)

        // Wait for the first event so connection/auth errors still get a proper status code.
        const first = await stream.next()
        res.statusCode = 200
        res.setHeader('Content-Type', 'application/x-ndjson; charset=utf-8')
        res.setHeader('Cache-Control', 'no-store')
        res.setHeader('X-Accel-Buffering', 'no')
        if (!first.done) res.write(JSON.stringify(first.value) + '\n')
        try {
          for await (const event of stream) res.write(JSON.stringify(event) + '\n')
        } catch (err) {
          if (!abort.signal.aborted) res.write(JSON.stringify({ type: 'error', message: describeError(err).message }) + '\n')
        }
        return res.end()
      }

      if (route === 'GET /api/fs/dirs')
        return sendJson(res, 200, await listDirs(url.searchParams.get('path'), url.searchParams.get('hidden') === '1'))

      if (route === 'POST /api/project/scan') {
        const body = await readObject(req)
        return sendJson(res, 200, await scanProject(str(body, 'root'), scanOptions(body.options)))
      }

      if (route === 'POST /api/project/read') {
        const body = await readObject(req)
        const files = await readFiles(str(body, 'root'), strList(body, 'paths', 5000), { condense: body.condense === true })
        return sendJson(res, 200, { files })
      }

      if (route === 'POST /api/project/apply') {
        const body = await readObject(req)
        return sendJson(res, 200, await applyChanges(str(body, 'root'), applyList(body.changes), body.backup !== false))
      }

      if (route === 'POST /api/project/undo') {
        const body = await readObject(req)
        return sendJson(res, 200, await undoApply(str(body, 'root'), str(body, 'backupId')))
      }

      if (route === 'GET /api/threads') return sendJson(res, 200, { threads: await listThreads(url.searchParams.get('root') ?? '') })

      const threadMatch = /^GET \/api\/threads\/([\w-]+)$/.exec(route)
      if (threadMatch) return sendJson(res, 200, await loadThread(url.searchParams.get('root') ?? '', threadMatch[1]))

      if (route === 'POST /api/threads/save') {
        const body = await readObject(req)
        const summary = isObj(body.summary) ? body.summary : {}
        await saveThread(
          str(body, 'root'),
          str(body, 'id'),
          {
            title: typeof summary.title === 'string' ? summary.title.slice(0, 200) : 'Conversation',
            updatedAt: new Date().toISOString(),
            turns: typeof summary.turns === 'number' ? summary.turns : 0,
          },
          body.thread,
        )
        return sendJson(res, 200, { ok: true })
      }

      if (route === 'POST /api/git/info') {
        const body = await readObject(req)
        return sendJson(res, 200, await gitInfo(str(body, 'root'), strList(body, 'paths', 2000)))
      }

      if (route === 'POST /api/pr/draft') {
        const cfg = requireConfig()
        const body = await readObject(req)
        const root = str(body, 'root')
        const paths = strList(body, 'paths', 2000)
        const info = await gitInfo(root, paths)
        const notes = typeof body.notes === 'string' ? body.notes.slice(0, 20_000) : ''
        const draft = await draftPullRequest(
          clientsFor(cfg).chat,
          cfg,
          { notes, diff: await diffFor(root, paths), branch: info.branch ?? '' },
          abort.signal,
        )
        return sendJson(res, 200, draft)
      }

      if (route === 'POST /api/pr/create') {
        const body = await readObject(req)
        return sendJson(
          res,
          200,
          await createPullRequest(str(body, 'root'), {
            paths: strList(body, 'paths', 2000),
            title: str(body, 'title'),
            body: typeof body.body === 'string' ? body.body : '',
            base: str(body, 'base'),
            draft: body.draft === true,
            newBranch: typeof body.newBranch === 'string' && body.newBranch.trim() ? body.newBranch.trim() : undefined,
          }),
        )
      }

      if (route === 'POST /api/threads/delete') {
        const body = await readObject(req)
        await deleteThread(str(body, 'root'), str(body, 'id'))
        return sendJson(res, 200, { ok: true })
      }

      throw new HttpError(404, `No route for ${route}.`)
    } catch (err) {
      if (abort.signal.aborted) return
      if (res.headersSent) return res.end()
      if (err instanceof HttpError || err instanceof PathError) return sendJson(res, err.status, { error: err.message })
      const { status, message } = describeError(err)
      return sendJson(res, status, { error: message })
    }
  }
}
