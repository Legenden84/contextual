import path from 'node:path'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'
import { contextualServer } from './src/server/plugin.ts'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss(), contextualServer()],
  resolve: {
    alias: {
      '@': path.resolve(import.meta.dirname, 'src/web'),
    },
  },
})
