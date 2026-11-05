// Mounts the /api routes on the Vite dev and preview servers. There is no separate backend:
// the app runs locally and this plugin is the Node side.

import { loadEnv, type Plugin } from 'vite'
import { createApi } from './api.ts'

export function contextualServer(): Plugin {
  // Load every variable from .env / .env.local (empty prefix), for server use only.
  // Vite still exposes only VITE_* variables to the browser bundle.
  let env: Record<string, string> = {}
  const api = createApi(() => env)

  return {
    name: 'contextual-server',
    configResolved(config) {
      const dir = typeof config.envDir === 'string' ? config.envDir : config.root
      env = loadEnv(config.mode, dir, '')
    },
    configureServer(server) {
      server.middlewares.use(api)
    },
    configurePreviewServer(server) {
      server.middlewares.use(api)
    },
  }
}
