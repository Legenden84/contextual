import { useCallback, useEffect, useRef, useState } from 'react'

/** Copies text to the clipboard and reports `copied` for a moment afterwards. */
export function useCopy(resetMs = 1200): [boolean, (text: string) => void] {
  const [copied, setCopied] = useState(false)
  const timer = useRef<number | undefined>(undefined)

  useEffect(() => () => window.clearTimeout(timer.current), [])

  const copy = useCallback(
    (text: string) => {
      void navigator.clipboard?.writeText(text).then(() => {
        setCopied(true)
        window.clearTimeout(timer.current)
        timer.current = window.setTimeout(() => setCopied(false), resetMs)
      })
    },
    [resetMs],
  )

  return [copied, copy]
}
