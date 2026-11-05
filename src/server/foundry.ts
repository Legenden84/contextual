// Talks to the Azure AI Foundry deployment through the openai SDK.
//
// Conversations use the Responses API when the deployment supports it: Azure stores each
// reply and the next request sends only `previous_response_id` plus the new turn. Otherwise
// (or if a stored response has expired) it falls back to chat completions with the full
// transcript, which the client keeps byte-for-byte stable so Azure's prompt cache applies.

import OpenAI, { AzureOpenAI } from 'openai'
import type { FoundryConfig } from './config.ts'

export type ChatRole = 'system' | 'user' | 'assistant'

/** Text, or text and pictures (user messages only). */
export type ContentPart = { type: 'text'; text: string } | { type: 'image_url'; image_url: { url: string; detail?: 'auto' | 'low' | 'high' } }

export type ChatMessage =
  | { role: 'system'; content: string }
  | { role: 'assistant'; content: string }
  | { role: 'user'; content: string | ContentPart[] }

/** One turn of a conversation. */
export interface ConversationRequest {
  /** Rules + output format; sent with every request. */
  system: string
  /** The earlier turns exactly as sent (user) and received (assistant). */
  history: ChatMessage[]
  /** The new user turn. */
  input: string | ContentPart[]
  /** Last stored Azure response of this conversation, if any. */
  previousResponseId: string | null
}

export type ApiKind = 'responses' | 'chat'

/** Events streamed to the browser, one JSON object per line. */
export type ChatEvent =
  | { type: 'delta'; text: string }
  | { type: 'usage'; inputTokens: number; outputTokens: number; cachedTokens: number }
  | { type: 'done'; finishReason: string | null; responseId: string | null; api: ApiKind; chainReset: boolean }
  | { type: 'error'; message: string }

export interface FoundryClients {
  /** Chat completions and the connection test (api-version from .env.local). */
  chat: AzureOpenAI
  /** Responses API on Azure's v1 endpoint (<endpoint>/openai/v1/, no api-version). */
  responses: OpenAI
}

export function createFoundryClients(cfg: FoundryConfig): FoundryClients {
  const base = cfg.endpoint.endsWith('/') ? cfg.endpoint : `${cfg.endpoint}/`
  return {
    chat: new AzureOpenAI({ endpoint: cfg.endpoint, apiKey: cfg.apiKey, apiVersion: cfg.apiVersion, deployment: cfg.deployment }),
    responses: new OpenAI({ baseURL: new URL('openai/v1/', base).toString(), apiKey: cfg.apiKey, defaultHeaders: { 'api-key': cfg.apiKey } }),
  }
}

/** Whether each endpoint+deployment supports the Responses API, learned on first use. */
const responsesSupport = new Map<string, boolean>()

const statusOf = (err: unknown) => (err && typeof err === 'object' && 'status' in err && typeof err.status === 'number' ? err.status : 0)
const messageOf = (err: unknown) => (err instanceof Error ? err.message : String(err))

/** The stored previous response is gone (expired, deleted, or from another resource). */
const isMissingPrevious = (err: unknown) => [400, 404].includes(statusOf(err)) && /previous[_ ]response/i.test(messageOf(err))

/** The deployment or API version doesn't offer the Responses API. */
const isUnsupported = (err: unknown) =>
  (statusOf(err) === 404 && !isMissingPrevious(err)) ||
  (statusOf(err) === 400 && /(api[- ]?version|not supported|unsupported|operation|responses)/i.test(messageOf(err)) && !isMissingPrevious(err))

type ResponsesInput = OpenAI.Responses.ResponseInput

function toResponsesInput(messages: ChatMessage[]): ResponsesInput {
  return messages
    .filter((m) => m.role !== 'system')
    .map((m) => {
      if (m.role === 'assistant') return { role: 'assistant' as const, content: m.content }
      if (typeof m.content === 'string') return { role: 'user' as const, content: m.content }
      return {
        role: 'user' as const,
        content: m.content.map((p) =>
          p.type === 'text'
            ? { type: 'input_text' as const, text: p.text }
            : { type: 'input_image' as const, image_url: p.image_url.url, detail: 'high' as const },
        ),
      }
    })
}

/** Starts a Responses API stream; throws before yielding if the request is rejected. */
async function startResponses(
  client: OpenAI,
  cfg: FoundryConfig,
  req: ConversationRequest,
  previousResponseId: string | null,
  signal: AbortSignal,
): Promise<AsyncGenerator<ChatEvent>> {
  const turns: ChatMessage[] = previousResponseId
    ? [{ role: 'user', content: req.input }]
    : [...req.history, { role: 'user', content: req.input }]
  const stream = await client.responses.create(
    {
      model: cfg.deployment,
      instructions: req.system,
      input: toResponsesInput(turns),
      previous_response_id: previousResponseId ?? undefined,
      store: true,
      stream: true,
      max_output_tokens: cfg.maxCompletionTokens,
    },
    { signal },
  )
  const chainReset = !!req.previousResponseId && !previousResponseId

  return (async function* () {
    let responseId: string | null = null
    let finishReason: string | null = null
    let usage: OpenAI.Responses.ResponseUsage | undefined
    for await (const ev of stream) {
      if (ev.type === 'response.output_text.delta') yield { type: 'delta', text: ev.delta }
      else if (ev.type === 'response.created') responseId = ev.response.id
      else if (ev.type === 'response.completed' || ev.type === 'response.incomplete') {
        responseId = ev.response.id
        usage = ev.response.usage ?? undefined
        const reason = ev.response.incomplete_details?.reason
        finishReason = ev.type === 'response.completed' ? 'stop' : reason === 'max_output_tokens' ? 'length' : (reason ?? 'incomplete')
      } else if (ev.type === 'response.failed') throw new Error(ev.response.error?.message ?? 'The model failed to respond.')
      else if (ev.type === 'error') throw new Error(ev.message)
    }
    if (usage)
      yield {
        type: 'usage',
        inputTokens: usage.input_tokens,
        outputTokens: usage.output_tokens,
        cachedTokens: usage.input_tokens_details?.cached_tokens ?? 0,
      }
    yield { type: 'done', finishReason, responseId, api: 'responses', chainReset }
  })()
}

/** Chat completions with the full transcript (fallback). */
async function* streamChat(client: AzureOpenAI, cfg: FoundryConfig, req: ConversationRequest, signal: AbortSignal): AsyncGenerator<ChatEvent> {
  const messages: ChatMessage[] = [{ role: 'system', content: req.system }, ...req.history, { role: 'user', content: req.input }]
  const stream = await client.chat.completions.create(
    {
      model: cfg.deployment,
      messages,
      max_completion_tokens: cfg.maxCompletionTokens,
      stream: true,
      stream_options: { include_usage: true },
    },
    { signal },
  )

  let finishReason: string | null = null
  for await (const chunk of stream) {
    const choice = chunk.choices[0]
    if (choice?.delta?.content) yield { type: 'delta', text: choice.delta.content }
    if (choice?.finish_reason) finishReason = choice.finish_reason
    if (chunk.usage)
      yield {
        type: 'usage',
        inputTokens: chunk.usage.prompt_tokens,
        outputTokens: chunk.usage.completion_tokens,
        cachedTokens: chunk.usage.prompt_tokens_details?.cached_tokens ?? 0,
      }
  }
  yield { type: 'done', finishReason, responseId: null, api: 'chat', chainReset: !!req.previousResponseId }
}

/**
 * One conversation turn. Prefers the Responses API (only the new turn is uploaded), retries
 * with the full transcript if the stored chain is gone, and falls back to chat completions
 * if the deployment doesn't support it. The model only returns text; nothing here touches disk.
 */
export async function* streamConversation(
  clients: FoundryClients,
  cfg: FoundryConfig,
  req: ConversationRequest,
  signal: AbortSignal,
): AsyncGenerator<ChatEvent> {
  const key = `${cfg.endpoint}|${cfg.deployment}`
  if (responsesSupport.get(key) !== false) {
    let stream: AsyncGenerator<ChatEvent> | null = null
    try {
      stream = await startResponses(clients.responses, cfg, req, req.previousResponseId, signal)
    } catch (err) {
      if (signal.aborted) throw err
      if (req.previousResponseId && isMissingPrevious(err)) {
        stream = await startResponses(clients.responses, cfg, req, null, signal)
      } else if (isUnsupported(err)) {
        responsesSupport.set(key, false)
        console.warn(`[contextual] Responses API not available (${messageOf(err)}); using chat completions.`)
      } else throw err
    }
    if (stream) {
      responsesSupport.set(key, true)
      yield* stream
      return
    }
  }
  yield* streamChat(clients.chat, cfg, req, signal)
}

/** Round-trip check for Settings → Test connection. Returns latency in ms. */
export async function testConnection(client: AzureOpenAI, cfg: FoundryConfig, signal: AbortSignal): Promise<number> {
  const started = performance.now()
  await client.chat.completions.create(
    { model: cfg.deployment, messages: [{ role: 'user', content: 'ping' }], max_completion_tokens: 16 },
    { signal },
  )
  return Math.round(performance.now() - started)
}

/** A readable message for SDK and network errors, without echoing secrets. */
export function describeError(err: unknown): { status: number; message: string } {
  if (err && typeof err === 'object' && 'status' in err && typeof err.status === 'number') {
    const msg = 'message' in err && typeof err.message === 'string' ? err.message : 'Request failed'
    return { status: err.status, message: `Foundry returned ${err.status}: ${msg}` }
  }
  if (err instanceof Error) return { status: 502, message: `Could not reach Foundry: ${err.message}` }
  return { status: 502, message: 'Could not reach Foundry.' }
}
