import type { SVGProps } from 'react'
import { cn } from '@/lib/utils'

// 16px codicon-like line icons, 1.3px stroke in currentColor.
const PATHS = {
  chevronRight: <path d="M6 4l4 4-4 4" />,
  chevronDown: <path d="M4 6l4 4 4-4" />,
  folder: <path d="M1.5 4.5a1 1 0 011-1h3.6l1.4 1.5h5.9a1 1 0 011 1v6.5a1 1 0 01-1 1h-10.9a1 1 0 01-1-1z" />,
  folderOpen: (
    <>
      <path d="M1.5 12.5v-8a1 1 0 011-1h3.6l1.4 1.5h5a1 1 0 011 1v1" />
      <path d="M1.5 12.5l2-5h11l-2 5z" />
    </>
  ),
  file: (
    <>
      <path d="M4 1.5h5l3.5 3.5v9.5h-8.5z" />
      <path d="M9 1.5v3.5h3.5" />
    </>
  ),
  copy: (
    <>
      <rect x="5.5" y="5.5" width="8" height="8" rx="1" />
      <path d="M3.5 10.5h-1v-8h8v1" />
    </>
  ),
  wrap: (
    <>
      <path d="M2 4h12M2 8h9.5a2 2 0 010 4H8" />
      <path d="M9.5 10.5L8 12l1.5 1.5M2 12h3" />
    </>
  ),
  send: (
    <>
      <path d="M2 8l11.5-5.5L11 14l-2.5-4.5z" />
      <path d="M8.5 9.5l5-7" />
    </>
  ),
  search: (
    <>
      <circle cx="7" cy="7" r="4.5" />
      <path d="M10.5 10.5l3.5 3.5" />
    </>
  ),
  refresh: (
    <>
      <path d="M13 8a5 5 0 11-1.5-3.6" />
      <path d="M13 2.5v2.5h-2.5" />
    </>
  ),
  warning: (
    <>
      <path d="M8 1.8l6.5 11.7h-13z" />
      <path d="M8 6.5v3M8 11.5v.2" />
    </>
  ),
  check: <path d="M3 8.5l3 3 7-7" />,
  minus: <path d="M4 8h8" />,
  plus: <path d="M8 3v10M3 8h10" />,
  close: <path d="M4 4l8 8M12 4l-8 8" />,
  gear: (
    <>
      <circle cx="8" cy="8" r="2" />
      <path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4" />
    </>
  ),
  undo: (
    <>
      <path d="M5 3L2 6l3 3" />
      <path d="M2 6h7.5a4 4 0 010 8H6" />
    </>
  ),
  unfold: <path d="M8 1.5v4M6 3.5l2 2 2-2M8 14.5v-4M6 12.5l2-2 2 2M2 8h12" />,
  eye: (
    <>
      <path d="M1 8s2.5-5 7-5 7 5 7 5-2.5 5-7 5-7-5-7-5z" />
      <circle cx="8" cy="8" r="2" />
    </>
  ),
  split: (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" rx="1" />
      <path d="M8 2.5v11" />
    </>
  ),
  unified: (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" rx="1" />
      <path d="M4 6h8M4 10h8" />
    </>
  ),
  back: <path d="M10 3L5 8l5 5" />,
  history: (
    <>
      <circle cx="8" cy="8" r="6" />
      <path d="M8 4.5V8l2.5 1.5" />
    </>
  ),
  sparkle: <path d="M8 1.5l1.3 4.2L13.5 7 9.3 8.3 8 12.5 6.7 8.3 2.5 7l4.2-1.3z" />,
  book: <path d="M2 3h4.5A1.5 1.5 0 018 4.5v9A1.5 1.5 0 006.5 12H2zM14 3H9.5A1.5 1.5 0 008 4.5v9A1.5 1.5 0 019.5 12H14z" />,
  files: (
    <>
      <path d="M5.5 1.5h4l3 3v7h-7z" />
      <path d="M9.5 1.5v3h3M3.5 4.5v10h6" />
    </>
  ),
  key: (
    <>
      <circle cx="5" cy="11" r="3" />
      <path d="M7.2 8.8l6.3-6.3M11.5 4.5l2 2" />
    </>
  ),
  filter: <path d="M2 3h12l-4.5 5.5v4.5l-3 1.5v-6z" />,
  external: <path d="M9 2.5h4.5V7M13.5 2.5L7.5 8.5M12 9.5v4h-9.5v-9.5h4" />,
  dots: (
    <>
      <circle cx="3.5" cy="8" r=".6" />
      <circle cx="8" cy="8" r=".6" />
      <circle cx="12.5" cy="8" r=".6" />
    </>
  ),
  trash: <path d="M2.5 4h11M6 4V2.5h4V4M4 4l.7 9.5h6.6L12 4" />,
  cloud: <path d="M4.5 12.5a3 3 0 01-.4-6 4 4 0 017.7-.9 3.2 3.2 0 01-.3 6.9z" />,
  branch: (
    <>
      <circle cx="4.5" cy="3.5" r="1.5" />
      <circle cx="4.5" cy="12.5" r="1.5" />
      <circle cx="11.5" cy="5.5" r="1.5" />
      <path d="M4.5 5v6M11.5 7c0 3-7 2-7 4" />
    </>
  ),
  sun: (
    <>
      <circle cx="8" cy="8" r="3" />
      <path d="M8 1v1.5M8 13.5V15M1 8h1.5M13.5 8H15M3 3l1 1M12 12l1 1M3 13l1-1M12 4l1-1" />
    </>
  ),
  moon: <path d="M13.5 9.5A5.5 5.5 0 016.5 2.5a5.5 5.5 0 107 7z" />,
} as const

export type IconName = keyof typeof PATHS

export function Icon({ name, className, ...rest }: { name: IconName } & SVGProps<SVGSVGElement>) {
  return (
    <svg
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.3}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={cn('inline-block size-4 flex-none', className)}
      {...rest}
    >
      {PATHS[name]}
    </svg>
  )
}
