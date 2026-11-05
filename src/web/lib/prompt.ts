// Builds what is sent to the model: rules + output format as the system prompt, and each new
// user turn with only the files the model doesn't have yet (new, changed, or "Resend all files").
// Pictures (pasted screenshots, ticked image files) go as image parts of the user turn.

import type { Rule } from '@/data/types'
import type { ApiContent, ContentPart } from './api'
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

/**
 * The new user turn. Only files that are new to this conversation or changed since they were
 * last sent are attached; the model already has the others from earlier turns.
 */
export function buildTurn(opts: {
  text: string
  /** New or changed files (attached in full). */
  files: ContextFile[]
  /** Still selected and unchanged since sent earlier in this conversation. */
  unchanged: string[]
  /** Sent earlier but no longer selected. */
  removed: string[]
  attachments: ContextImage[]
}): ApiContent {
  const sections: string[] = []
  if (opts.files.length) sections.push(contextBlock(opts.files))
  if (opts.unchanged.length)
    sections.push(
      `# Unchanged files\n\nStill in context, unchanged since sent earlier in this conversation:\n${opts.unchanged.map((p) => `- ${p}`).join('\n')}`,
    )
  if (opts.removed.length)
    sections.push(`# Removed from context\n\nIgnore these files from earlier messages:\n${opts.removed.map((p) => `- ${p}`).join('\n')}`)
  const body = sections.length ? `${sections.join('\n\n')}\n\n# Request\n\n${opts.text}` : opts.text
  const images = [
    ...opts.files.filter((f) => f.image != null).map((f) => ({ name: f.path, url: f.image! })),
    ...opts.attachments,
  ]
  return withImages(body, images)
}

/** Plain text of a turn, for token estimates. */
export function contentText(content: ApiContent): string {
  return typeof content === 'string' ? content : content.map((p) => (p.type === 'text' ? p.text : '')).join('\n')
}

/** Pictures in a turn. */
export const contentImages = (content: ApiContent): number =>
  typeof content === 'string' ? 0 : content.filter((p) => p.type === 'image_url').length
