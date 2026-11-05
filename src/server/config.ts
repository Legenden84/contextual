// Azure AI Foundry settings, read from .env.local (loaded by the Vite plugin).
// Variable names follow the `openai` SDK's AzureOpenAI defaults where one exists.

export interface FoundryConfig {
  /** Resource endpoint, e.g. https://pdaazuregpt.openai.azure.com/ */
  endpoint: string
  apiKey: string
  apiVersion: string
  /** Deployment name, sent as `model` on every request. */
  deployment: string
  /** Display name of the underlying model. */
  modelName: string
  maxCompletionTokens: number
  /** Context window used by the context bar. Unset until you confirm it for the model. */
  contextLimit?: number
  /** USD per 1M input / output tokens, for cost estimates. */
  inputPricePer1M?: number
  outputPricePer1M?: number
}

export const REQUIRED_ENV = [
  'AZURE_OPENAI_ENDPOINT',
  'AZURE_OPENAI_API_KEY',
  'OPENAI_API_VERSION',
  'AZURE_OPENAI_DEPLOYMENT',
] as const

export type ConfigResult =
  | { ok: true; config: FoundryConfig }
  | { ok: false; missing: string[]; invalid: string[] }

type Env = Record<string, string | undefined>

function positive(env: Env, key: string, invalid: string[]): number | undefined {
  const raw = env[key]?.trim()
  if (!raw) return undefined
  const n = Number(raw)
  if (!Number.isFinite(n) || n <= 0) {
    invalid.push(key)
    return undefined
  }
  return n
}

export function readFoundryConfig(env: Env): ConfigResult {
  const value = (k: string) => env[k]?.trim() || undefined
  const missing = REQUIRED_ENV.filter((k) => !value(k))
  const invalid: string[] = []

  const endpoint = value('AZURE_OPENAI_ENDPOINT')
  if (endpoint) {
    try {
      // HTTPS only, except a local mock or proxy on this machine.
      const url = new URL(endpoint)
      const local = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname)
      if (url.protocol !== 'https:' && !(local && url.protocol === 'http:')) invalid.push('AZURE_OPENAI_ENDPOINT')
    } catch {
      invalid.push('AZURE_OPENAI_ENDPOINT')
    }
  }

  const maxCompletionTokens = positive(env, 'AZURE_OPENAI_MAX_COMPLETION_TOKENS', invalid) ?? 16384
  const contextLimit = positive(env, 'AZURE_OPENAI_CONTEXT_LIMIT', invalid)
  const inputPricePer1M = positive(env, 'AZURE_OPENAI_INPUT_PRICE_PER_1M', invalid)
  const outputPricePer1M = positive(env, 'AZURE_OPENAI_OUTPUT_PRICE_PER_1M', invalid)

  if (missing.length || invalid.length) return { ok: false, missing, invalid }

  const deployment = value('AZURE_OPENAI_DEPLOYMENT')!
  return {
    ok: true,
    config: {
      endpoint: endpoint!,
      apiKey: value('AZURE_OPENAI_API_KEY')!,
      apiVersion: value('OPENAI_API_VERSION')!,
      deployment,
      modelName: value('AZURE_OPENAI_MODEL_NAME') ?? deployment,
      maxCompletionTokens,
      contextLimit,
      inputPricePer1M,
      outputPricePer1M,
    },
  }
}
