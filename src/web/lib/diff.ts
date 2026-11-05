// Line diff (LCS) with word-level ranges and hunk collapsing, GitHub-style.
// Ported from docs/design/contextual/components/bundle.js.

export type OpKind = ' ' | '+' | '-'

export interface DiffOp {
  k: OpKind
  /** 1-based line number in the old file. */
  a?: number
  /** 1-based line number in the new file. */
  b?: number
  t: string
  /** [start, end) of the changed characters when paired with a counterpart line. */
  r?: [number, number]
}

export interface Diff {
  ops: DiffOp[]
  adds: number
  dels: number
  oldLines: string[]
  newLines: string[]
}

export type DiffItem = { kind: 'hunk'; lines: DiffOp[] } | { kind: 'gap'; lines: DiffOp[] }

function lcsOps(a: string[], b: string[]): DiffOp[] {
  const n = a.length
  const m = b.length
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0))
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--) dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1])

  const ops: DiffOp[] = []
  let i = 0
  let j = 0
  while (i < n || j < m) {
    if (i < n && j < m && a[i] === b[j]) {
      ops.push({ k: ' ', a: i + 1, b: j + 1, t: a[i] })
      i++
      j++
    } else if (j < m && (i >= n || dp[i][j + 1] >= dp[i + 1][j])) {
      ops.push({ k: '+', b: j + 1, t: b[j] })
      j++
    } else {
      ops.push({ k: '-', a: i + 1, t: a[i] })
      i++
    }
  }

  // Inside each change run, put removals before additions.
  const out: DiffOp[] = []
  let run: DiffOp[] = []
  const flushRun = () => {
    out.push(...run.filter((o) => o.k === '-'), ...run.filter((o) => o.k === '+'))
    run = []
  }
  for (const o of ops) {
    if (o.k === ' ') {
      flushRun()
      out.push(o)
    } else run.push(o)
  }
  flushRun()
  return out
}

/** Pairs removed/added lines and marks the changed span when the lines are similar enough. */
function wordRanges(ops: DiffOp[]): DiffOp[] {
  for (let i = 0; i < ops.length; i++) {
    if (ops[i].k !== '-') continue
    let d = i
    while (d < ops.length && ops[d].k === '-') d++
    let p = d
    while (p < ops.length && ops[p].k === '+') p++
    const dels = ops.slice(i, d)
    const adds = ops.slice(d, p)
    for (let q = 0; q < Math.min(dels.length, adds.length); q++) {
      const x = dels[q].t
      const y = adds[q].t
      let pre = 0
      let suf = 0
      while (pre < x.length && pre < y.length && x[pre] === y[pre]) pre++
      while (suf < x.length - pre && suf < y.length - pre && x[x.length - 1 - suf] === y[y.length - 1 - suf]) suf++
      const common = pre + suf
      const longest = Math.max(x.trim().length, y.trim().length)
      if (common - (x.length - x.trimStart().length) > longest * 0.35) {
        dels[q].r = [pre, x.length - suf]
        adds[q].r = [pre, y.length - suf]
      }
    }
    i = p - 1
  }
  return ops
}

export function computeDiff(oldText: string, newText: string): Diff {
  const a = oldText ? oldText.split('\n') : []
  const b = newText ? newText.split('\n') : []
  const ops = wordRanges(lcsOps(a, b))
  return {
    ops,
    adds: ops.filter((o) => o.k === '+').length,
    dels: ops.filter((o) => o.k === '-').length,
    oldLines: a,
    newLines: b,
  }
}

/** Groups ops into visible hunks (changes plus `context` lines around them) and collapsible gaps. */
export function makeHunks(ops: DiffOp[], context = 3): DiffItem[] {
  const keep = ops.map(() => false)
  ops.forEach((o, i) => {
    if (o.k === ' ') return
    for (let k = Math.max(0, i - context); k <= Math.min(ops.length - 1, i + context); k++) keep[k] = true
  })
  const items: DiffItem[] = []
  let i = 0
  while (i < ops.length) {
    const kind = keep[i] ? 'hunk' : 'gap'
    const lines: DiffOp[] = []
    while (i < ops.length && keep[i] === (kind === 'hunk')) lines.push(ops[i++])
    items.push({ kind, lines })
  }
  return items
}

const MEMBER = /^\s*(public|internal|private|protected)\b.*\b(class|record|struct|interface|void|Task|static)\b/

/** `@@ -a,n +b,m @@ enclosing member` for a hunk. */
export function hunkHeader(lines: DiffOp[], newLines: string[]): string {
  let aStart = 0
  let bStart = 0
  let aCount = 0
  let bCount = 0
  for (const o of lines) {
    if (o.a) {
      aStart ||= o.a
      aCount++
    }
    if (o.b) {
      bStart ||= o.b
      bCount++
    }
  }
  const from = lines[0].b || bStart || 1
  let member = ''
  for (let k = from - 2; k >= 0; k--) {
    if (MEMBER.test(newLines[k] ?? '')) {
      member = newLines[k].trim()
      break
    }
  }
  return `@@ -${aStart},${aCount} +${bStart},${bCount} @@${member ? ' ' + member : ''}`
}

/** A plain-text unified diff, for "Copy diff". */
export function unifiedPatch(path: string, diff: Diff, status: 'A' | 'M' | 'D'): string {
  const out = [`--- ${status === 'A' ? '/dev/null' : 'a/' + path}`, `+++ ${status === 'D' ? '/dev/null' : 'b/' + path}`]
  for (const item of makeHunks(diff.ops, 3)) {
    if (item.kind === 'gap') continue
    out.push(hunkHeader(item.lines, diff.newLines))
    for (const o of item.lines) out.push(o.k + o.t)
  }
  return out.join('\n')
}
