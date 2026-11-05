// Thin wrapper over the openai SDK's AzureOpenAI client (same client as the Foundry samples).

import { AzureOpenAI } from 'openai'
import type { FoundryConfig } from './config.ts'

export type ChatRole = 'system' | 'user' | 'assistant'

/** Text, or text and pictures (user messages only). */
export type ContentPart = { type: 'text'; text: string } | { type: 'image_url'; image_url: { url: string; detail?: 'auto' | 'low' | 'high' } }

export type ChatMessage =
  | { role: 'system'; content: string }
  | { role: 'assistant'; content: string }
  | { role: 'user'; content: string | ContentPart[] }

/** Events streamed to the browser, one JSON object per line. */
export type ChatEvent =
  | { type: 'delta'; text: string }
  | { type: 'usage'; inputTokens: number; outputTokens: number }
  | { type: 'done'; finishReason: string | null }
  | { type: 'error'; message: string }

export function createFoundryClient(cfg: FoundryConfig): AzureOpenAI {
  return new AzureOpenAI({
    endpoint: cfg.endpoint,
    apiKey: cfg.apiKey,
    apiVersion: cfg.apiVersion,
    deployment: cfg.deployment,
  })
}

/** Streams a chat completion. The model only returns text; nothing here touches disk. */
export async function* streamChat(
  client: AzureOpenAI,
  cfg: FoundryConfig,
  messages: ChatMessage[],
  signal: AbortSignal,
): AsyncGenerator<ChatEvent> {
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
      yield { type: 'usage', inputTokens: chunk.usage.prompt_tokens, outputTokens: chunk.usage.completion_tokens }
  }
  yield { type: 'done', finishReason }
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
