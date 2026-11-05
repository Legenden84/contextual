// HTTP routes under /api, mounted as Connect middleware on the local Vite server.
//
//   GET  /api/health      Configuration status (never the API key).
//   POST /api/test        Round-trip to the deployment, returns latency.
//   POST /api/chat        { messages } → NDJSON stream of ChatEvent.

import type { IncomingMessage, ServerResponse } from 'node:http'
import type { AzureOpenAI } from 'openai'
import { readFoundryConfig, type ConfigResult, type FoundryConfig } from './config.ts'
import { createFoundryClient, describeError, streamChat, testConnection, type ChatMessage } from './foundry.ts'

type Env = Record<string, string | undefined>
type Next = (err?: unknown) => void

const MAX_BODY_BYTES = 8 * 1024 * 1024
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

function parseMessages(body: unknown): ChatMessage[] {
  const list = (body as { messages?: unknown })?.messages
  if (!Array.isArray(list) || list.length === 0) throw new HttpError(400, '`messages` must be a non-empty array.')
  return list.map((m, i) => {
    const { role, content } = (m ?? {}) as { role?: unknown; content?: unknown }
    if (typeof role !== 'string' || !ROLES.has(role) || typeof content !== 'string')
      throw new HttpError(400, `messages[${i}] needs a role (system, user, assistant) and string content.`)
    return { role: role as ChatMessage['role'], content }
  })
}

function publicConfig(result: ConfigResult) {
  if (!result.ok) return { configured: false, missing: result.missing, invalid: result.invalid }
  const { apiKey: _secret, ...rest } = result.config
  void _secret
  return { configured: true, ...rest }
}

/** Builds the /api middleware. `getEnv` is read per request so .env.local edits apply after a restart. */
export function createApi(getEnv: () => Env) {
  let cached: { key: string; client: AzureOpenAI } | null = null

  const clientFor = (cfg: FoundryConfig) => {
    const key = JSON.stringify([cfg.endpoint, cfg.apiKey, cfg.apiVersion, cfg.deployment])
    if (cached?.key !== key) cached = { key, client: createFoundryClient(cfg) }
    return cached.client
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
        const ms = await testConnection(clientFor(cfg), cfg, abort.signal)
        return sendJson(res, 200, { ok: true, latencyMs: ms, deployment: cfg.deployment })
      }

      if (route === 'POST /api/chat') {
        const cfg = requireConfig()
        const messages = parseMessages(await readJson(req))
        const stream = streamChat(clientFor(cfg), cfg, messages, abort.signal)

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

      throw new HttpError(404, `No route for ${route}.`)
    } catch (err) {
      if (abort.signal.aborted) return
      if (res.headersSent) return res.end()
      if (err instanceof HttpError) return sendJson(res, err.status, { error: err.message })
      const { status, message } = describeError(err)
      return sendJson(res, status, { error: message })
    }
  }
}
