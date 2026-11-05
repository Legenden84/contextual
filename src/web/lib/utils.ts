import { type ClassValue, clsx } from 'clsx'
import { extendTailwindMerge } from 'tailwind-merge'

// Teach tailwind-merge the design-token scales, so `text-ui-body` (a size) and
// `text-ink` (a colour) don't cancel each other out.
const twMerge = extendTailwindMerge({
  extend: {
    theme: {
      text: ['ui-caption', 'ui-small', 'ui-body', 'ui-prose', 'ui-title', 'ui-display', 'code', 'code-small'],
      shadow: ['popover', 'modal', 'window'],
      spacing: ['row', 'control'],
    },
  },
})

/** Merge conditional class names and resolve Tailwind conflicts. */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}
