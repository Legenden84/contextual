// Builds the messages sent to the model: rules + output format as the system prompt,
// the conversation so far, and (with "Resend full context") the selected files.

import type { Rule } from '@/data/sample'
import type { ApiMessage } from './api'
import { langFromPath } from './syntax'

export interface ContextFile {
  path: string
  /** Undefined when the contents aren't available to the app. */
  content?: string
}

const FENCE_LANG: Partial<Record<ReturnType<typeof langFromPath>, string>> = {
  csharp: 'csharp',
  typescript: 'typescript',
  python: 'python',
  json: 'json',
  yaml: 'yaml',
  xml: 'xml',
  sql: 'sql',
  markdown: 'markdown',
  shell: 'bash',
}

export function systemPrompt(rules: Rule[], outputFormat: string): string {
  const parts = rules.map((r) => `## ${r.path}\n\n${r.content.trim()}`)
  return [parts.length ? `# Project rules\n\n${parts.join('\n\n')}` : '', outputFormat.trim()].filter(Boolean).join('\n\n')
}

/** The files block prepended to a user message when the full context is (re)sent. */
export function contextBlock(files: ContextFile[]): string {
  const withContent = files.filter((f) => f.content != null)
  const missing = files.filter((f) => f.content == null)
  const blocks = withContent.map((f) => {
    const lang = FENCE_LANG[langFromPath(f.path)] ?? ''
    return `### FILE: ${f.path}\n\`\`\`${lang}\n${f.content}\n\`\`\``
  })
  const lines = [`# Context files (${files.length})`, '', ...blocks]
  if (missing.length)
    lines.push('', 'Also selected, contents not available:', ...missing.map((f) => `- ${f.path}`))
  return lines.join('\n')
}

export function buildMessages(opts: {
  system: string
  history: ApiMessage[]
  text: string
  files: ContextFile[] | null
}): ApiMessage[] {
  const user = opts.files ? `${contextBlock(opts.files)}\n\n# Request\n\n${opts.text}` : opts.text
  return [{ role: 'system', content: opts.system }, ...opts.history, { role: 'user', content: user }]
}
