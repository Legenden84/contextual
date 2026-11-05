// Builds the messages sent to the model: rules + output format as the system prompt,
// the conversation so far, and (with "Resend full context") the selected files.
// Pictures (pasted screenshots, ticked image files) go as image parts of the user message.

import type { Rule } from '@/data/sample'
import type { ApiContent, ApiMessage, ContentPart } from './api'
import { langFromPath } from './syntax'

export interface ContextFile {
  path: string
  /** Undefined when the contents aren't available to the app. */
  content?: string
  /** Image files: data URL sent as a picture. */
  image?: string
  /** A condensed copy of a large HTML/SVG file: a read-only reference. */
  condensed?: boolean
}

export interface ContextImage {
  name: string
  url: string
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

/** Header used for condensed references; lib/changes.ts never turns these into changes. */
export const REFERENCE_NOTE = '(condensed reference, read-only: do not return this file)'

export function systemPrompt(rules: Rule[], outputFormat: string): string {
  const parts = rules.map((r) => `## ${r.path}\n\n${r.content.trim()}`)
  return [parts.length ? `# Project rules\n\n${parts.join('\n\n')}` : '', outputFormat.trim()].filter(Boolean).join('\n\n')
}

/** The files block prepended to a user message when the full context is (re)sent. */
export function contextBlock(files: ContextFile[]): string {
  const text = files.filter((f) => f.content != null)
  const images = files.filter((f) => f.image != null)
  const missing = files.filter((f) => f.content == null && f.image == null)
  const blocks = text.map((f) => {
    const lang = FENCE_LANG[langFromPath(f.path)] ?? (/\.(html?|svg)$/i.test(f.path) ? 'html' : '')
    const header = f.condensed ? `### FILE: ${f.path} ${REFERENCE_NOTE}` : `### FILE: ${f.path}`
    const note = f.condensed
      ? 'Embedded images, fonts, minified scripts/styles and long path data were replaced by placeholders.\n'
      : ''
    return `${header}\n${note}\`\`\`${lang}\n${f.content}\n\`\`\``
  })
  const lines = [`# Context files (${files.length})`, '', ...blocks]
  if (images.length) lines.push('', 'Image files (attached as pictures below):', ...images.map((f) => `- ${f.path}`))
  if (missing.length) lines.push('', 'Also selected, contents not available:', ...missing.map((f) => `- ${f.path}`))
  return lines.join('\n')
}

/** Text plus pictures, each picture introduced by its name so the model can refer to it. */
export function withImages(text: string, images: ContextImage[]): ApiContent {
  if (!images.length) return text
  const parts: ContentPart[] = [{ type: 'text', text }]
  for (const img of images) {
    parts.push({ type: 'text', text: `Image: ${img.name}` })
    parts.push({ type: 'image_url', image_url: { url: img.url, detail: 'high' } })
  }
  return parts
}

export function buildMessages(opts: {
  system: string
  history: ApiMessage[]
  text: string
  files: ContextFile[] | null
  attachments: ContextImage[]
}): ApiMessage[] {
  const files = opts.files ?? []
  const body = opts.files ? `${contextBlock(files)}\n\n# Request\n\n${opts.text}` : opts.text
  const images = [
    ...files.filter((f) => f.image != null).map((f) => ({ name: f.path, url: f.image! })),
    ...opts.attachments,
  ]
  return [{ role: 'system', content: opts.system }, ...opts.history, { role: 'user', content: withImages(body, images) }]
}
