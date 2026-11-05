import type { FoundryStatus } from '@/state/app-context'

export interface FoundryLabel {
  text: string
  tone: 'ok' | 'warn' | 'muted'
  detail?: string
}

/** Short status text for the status bar and welcome screen. */
export function describeFoundry(status: FoundryStatus): FoundryLabel {
  switch (status.state) {
    case 'checking':
      return { text: 'Foundry: checking…', tone: 'muted' }
    case 'ready':
      return { text: `Foundry: ${status.health.deployment}`, tone: 'ok', detail: status.health.endpoint }
    case 'unconfigured': {
      const missing = [...status.missing, ...status.invalid]
      return {
        text: 'Foundry: not configured',
        tone: 'warn',
        detail: `Set ${missing.join(', ')} in .env.local and restart npm run dev.`,
      }
    }
    case 'offline':
      return { text: 'Foundry: local server unavailable', tone: 'warn', detail: status.error }
  }
}
