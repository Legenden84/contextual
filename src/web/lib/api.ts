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
    }
  | { configured: false; missing: string[]; invalid: string[] }

export type ChatRole = 'system' | 'user' | 'assistant'

export interface ApiMessage {
  role: ChatRole
  content: string
}

export type ChatEvent =
  | { type: 'delta'; text: string }
  | { type: 'usage'; inputTokens: number; outputTokens: number }
  | { type: 'done'; finishReason: string | null }
  | { type: 'error'; message: string }

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
  messages: ApiMessage[],
  onEvent: (e: ChatEvent) => void,
  signal: AbortSignal,
): Promise<void> {
  const res = await fetch('/api/chat', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ messages }),
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
