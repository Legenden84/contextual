/** 1.5k, 12k, 940 — for trees and lists. */
export function fmtK(n: number): string {
  if (n < 1000) return String(n)
  return (n / 1000).toFixed(n >= 10000 ? 0 : 1).replace(/\.0$/, '') + 'k'
}

/** 18,400 — for the context bar. */
export function fmtN(n: number): string {
  return n.toLocaleString('en-US')
}

/** Strips the common leading indentation of a template literal. */
export function dedent(s: string): string {
  const lines = s.replace(/^\n/, '').replace(/\s+$/, '').split('\n')
  let min = Infinity
  for (const l of lines) if (l.trim()) min = Math.min(min, l.match(/^ */)![0].length)
  return lines.map((l) => l.slice(min)).join('\n')
}

export function splitPath(path: string): { dir: string; name: string } {
  const i = path.lastIndexOf('/')
  return { dir: path.slice(0, i + 1), name: path.slice(i + 1) }
}

/** Rough token estimate for text the app hasn't counted yet (about 4 characters per token). */
export function estimateTokens(text: string): number {
  return Math.max(1, Math.round(text.length / 4))
}
