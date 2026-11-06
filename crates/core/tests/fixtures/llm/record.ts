// Records what the old server (foundry.ts on the openai SDK) sends and emits, for the parity test
// in crates/core/tests/llm.rs:
//
//   node crates/core/tests/fixtures/llm/record.ts --out crates/core/tests/fixtures/llm/expected.json
//
// A local HTTP server answers like Azure, routed by deployment (`model`) — see `route()`, which
// the Rust test mirrors — with the recorded-shape payloads in this folder. The output lists every
// request (method, path + query, auth/content headers, JSON body) and every result/event.

import { readFileSync, writeFileSync } from 'node:fs'
import http from 'node:http'
import type { AddressInfo } from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import type { FoundryConfig } from '../../../../../src/server/config.ts'
import {
  createFoundryClients,
  describeError,
  draftCommitMessage,
  draftPullRequest,
  repairFile,
  streamConversation,
  testConnection,
  type ConversationRequest,
} from '../../../../../src/server/foundry.ts'

const here = path.dirname(fileURLToPath(import.meta.url))
const file = (name: string) => readFileSync(path.join(here, name), 'utf8')
const canned = JSON.parse(file('canned.json')) as Record<string, unknown>

interface Reply {
  status: number
  type: string
  body: string
}
const json = (status: number, name: string): Reply => ({ status, type: 'application/json', body: JSON.stringify(canned[name]) })
const sse = (name: string): Reply => ({ status: 200, type: 'text/event-stream', body: file(name) })

function route(url: string, body: Record<string, unknown>): Reply {
  const model = body.model
  if (model === 'gpt-401') return json(401, 'unauthorized')
  if (url.startsWith('/openai/v1/responses')) {
    if (model === 'gpt-chat') return json(404, 'notFound')
    if (body.previous_response_id === 'resp_gone') return json(400, 'previousNotFound')
    if (model === 'gpt-fail') return sse('responses-failed.sse')
    if (model === 'gpt-incomplete') return sse('responses-incomplete.sse')
    return sse('responses-stream.sse')
  }
  if (body.stream) return sse('chat-stream.sse')
  const messages = body.messages as { content: string }[]
  const system = messages[0].content
  if (system.startsWith('You write git commit')) return json(200, 'commit')
  if (system.startsWith('You write GitHub pull request')) return json(200, 'pr')
  if (system.startsWith('You apply edits')) return json(200, messages[1].content.includes('CUT') ? 'repairCut' : 'repair')
  return json(200, 'ping')
}

const requests: unknown[] = []
const server = http.createServer(async (req, res) => {
  let raw = ''
  for await (const chunk of req) raw += chunk
  const body = raw ? (JSON.parse(raw) as Record<string, unknown>) : {}
  const h = req.headers
  requests.push({
    method: req.method,
    url: req.url,
    headers: { 'api-key': h['api-key'] ?? null, authorization: h.authorization ?? null, 'content-type': h['content-type'] ?? null },
    body,
  })
  const r = route(req.url ?? '', body)
  res.writeHead(r.status, { 'content-type': r.type })
  res.end(r.body)
})
await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
const port = (server.address() as AddressInfo).port

const cfg = (deployment: string): FoundryConfig => ({
  endpoint: `http://127.0.0.1:${port}/`,
  apiKey: 'test-key',
  apiVersion: '2025-04-01-preview',
  deployment,
  modelName: deployment,
  maxCompletionTokens: 4096,
})
const signal = () => new AbortController().signal

async function chat(deployment: string, req: ConversationRequest) {
  const c = cfg(deployment)
  const events: unknown[] = []
  try {
    for await (const e of streamConversation(createFoundryClients(c), c, req, signal())) events.push(e)
  } catch (err) {
    events.push({ type: 'error', message: describeError(err).message })
  }
  return events
}

async function call<T>(f: () => Promise<T>) {
  try {
    return { ok: await f() }
  } catch (err) {
    return { error: describeError(err).message }
  }
}

const image = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg=='
const history: ConversationRequest['history'] = [
  { role: 'user', content: [{ type: 'text', text: 'Files:\n\nsrc/a.ts' }, { type: 'image_url', image_url: { url: image, detail: 'high' } }] },
  { role: 'assistant', content: 'Earlier reply' },
]

const results: Record<string, unknown> = {}
results.fresh = await chat('gpt-resp', { system: 'SYS', history: [], input: [{ type: 'text', text: 'Look' }, { type: 'image_url', image_url: { url: image, detail: 'high' } }], previousResponseId: null })
results.fullHistory = await chat('gpt-resp', { system: 'SYS', history, input: 'Again', previousResponseId: null })
results.chained = await chat('gpt-resp', { system: 'SYS', history, input: 'Next', previousResponseId: 'resp_prev1' })
results.chainReset = await chat('gpt-resp', { system: 'SYS', history, input: 'After expiry', previousResponseId: 'resp_gone' })
results.fallback = await chat('gpt-chat', { system: 'SYS', history, input: 'Hello', previousResponseId: 'resp_prev1' })
results.fallbackAgain = await chat('gpt-chat', { system: 'SYS', history: [], input: 'Hello again', previousResponseId: null })
results.failed = await chat('gpt-fail', { system: 'SYS', history: [], input: 'x', previousResponseId: null })
results.incomplete = await chat('gpt-incomplete', { system: 'SYS', history: [], input: 'x', previousResponseId: null })
results.unauthorized = await chat('gpt-401', { system: 'SYS', history: [], input: 'x', previousResponseId: null })

const chatClient = (d: string) => createFoundryClients(cfg(d)).chat
results.test = await call(async () => typeof (await testConnection(chatClient('gpt-resp'), cfg('gpt-resp'), signal())))
results.test401 = await call(() => testConnection(chatClient('gpt-401'), cfg('gpt-401'), signal()))
results.draftCommit = await call(() => draftCommitMessage(chatClient('gpt-resp'), cfg('gpt-resp'), 'diff --git a/x b/x\n+1', signal()))
results.draftPr = await call(() =>
  draftPullRequest(chatClient('gpt-resp'), cfg('gpt-resp'), { notes: '', diff: 'diff --git a/x b/x', branch: 'feature/x', commits: '- Add x' }, signal()),
)
results.repair = await call(() =>
  repairFile(chatClient('gpt-resp'), cfg('gpt-resp'), { path: 'src/a.ts', original: 'export const a = 1\n', edits: '<<<<<<< SEARCH\na = 1\n=======\na = 2\n>>>>>>> REPLACE' }, signal()),
)
results.repairCut = await call(() => repairFile(chatClient('gpt-resp'), cfg('gpt-resp'), { path: 'src/a.ts', original: 'x', edits: 'CUT' }, signal()))

server.close()
const out = JSON.stringify({ requests, results }, null, 2) + '\n'
const at = process.argv.indexOf('--out')
if (at > 0) writeFileSync(process.argv[at + 1], out)
else process.stdout.write(out)
