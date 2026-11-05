// Defaults: the output-format system prompt, exclusion globs, and fixed token estimates.

import { dedent } from '@/lib/format'
import type { Deployment } from './types'

/** System prompt that makes the model return whole files under `### FILE:` headers (editable in Settings). */
export const outputPrompt = dedent(`
  You are helping edit files in a local project. The user reviews every change
  and applies it manually. You cannot write to disk.

  ## Output format
  - Explain the change in a few sentences first.
  - For every file you add or change, return its COMPLETE new contents.
    Never return partial snippets, "..." or "unchanged" placeholders.
  - Put each file in its own fenced block, directly after a header line:

  ### FILE: src/Path/Relative/To/Root.cs
  \`\`\`csharp
  <full file contents>
  \`\`\`

  - To delete a file, write a single line: ### DELETE: src/Path/To/File.cs
  - Only touch files that are included in the context.
  - Follow the project rules provided above this prompt.`)

/** Default exclusion globs (editable in Settings). */
export const defaultExclusions = [
  '**/node_modules/**',
  '**/.venv/**',
  '**/venv/**',
  '**/__pycache__/**',
  '**/bin/**',
  '**/obj/**',
  '**/dist/**',
  '**/build/**',
  '**/.git/**',
  'package-lock.json',
  'yarn.lock',
  'pnpm-lock.yaml',
  'poetry.lock',
  'Pipfile.lock',
  'packages.lock.json',
  'Cargo.lock',
]

/** Shown until the local server reports the deployment from .env.local. */
export const PLACEHOLDER_DEPLOYMENT: Deployment = { name: 'not configured', limit: 128000, inPrice: 0, outPrice: 0 }

/** Estimated tokens of a typical user message, for the context bar. */
export const USER_MESSAGE_TOKENS = 96
/** Output tokens assumed for the per-message cost estimate. */
export const EST_OUTPUT_TOKENS = 1800
