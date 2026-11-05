import { useState } from 'react'
import type { Lang } from '@/lib/syntax'
import { CodeView } from './CodeView'

/**
 * Editable code field: a transparent textarea over the highlighted CodeView, so typing
 * keeps the same rendering (gutter, colours) as every other code surface.
 */
export function CodeEditor({
  value,
  onChange,
  lang,
  label,
}: {
  value: string
  onChange: (v: string) => void
  lang: Lang
  label: string
}) {
  const [current, setCurrent] = useState<number | undefined>(undefined)
  const lineCount = value.split('\n').length

  const trackLine = (el: HTMLTextAreaElement) =>
    setCurrent(el.value.slice(0, el.selectionStart).split('\n').length)

  return (
    <div className="relative overflow-x-auto">
      <CodeView code={value} lang={lang} guides={false} currentLine={current} className="overflow-visible" />
      <textarea
        aria-label={label}
        spellCheck={false}
        value={value}
        onChange={(e) => {
          onChange(e.target.value)
          trackLine(e.target)
        }}
        onSelect={(e) => trackLine(e.currentTarget)}
        onBlur={() => setCurrent(undefined)}
        style={{ height: lineCount * 20 + 8 }}
        className="absolute inset-0 w-full resize-none overflow-hidden bg-transparent py-1 pr-4 pl-12 font-mono text-code whitespace-pre text-transparent caret-ink outline-none [font-variant-ligatures:none] selection:bg-editor-selection"
      />
    </div>
  )
}
