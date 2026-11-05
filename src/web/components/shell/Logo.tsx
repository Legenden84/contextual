/** Two-sheet mark: accent sheet, outlined sheet, success lines. */
export function Logo({ size = 16 }: { size?: number }) {
  return (
    <svg viewBox="0 0 16 16" width={size} height={size} aria-hidden="true" className="flex-none">
      <rect x="1" y="1.5" width="9" height="11" rx="2" className="fill-accent" />
      <rect x="6" y="3.5" width="9" height="11" rx="2" className="fill-surface-100 stroke-ink-muted" strokeWidth={1.2} />
      <path d="M8.5 7.5h4M8.5 10h2.5" className="fill-none stroke-success" strokeWidth={1.4} strokeLinecap="round" />
    </svg>
  )
}
