import type { TreeNode } from '@/data/sample'

export type CheckState = 'on' | 'off' | 'mixed'

export interface NodeStats {
  /** Selectable (not excluded) files below this node. */
  files: number
  /** Selected files below this node. */
  selected: number
  /** Tokens of the selected files below this node. */
  selectedTokens: number
}

export function nodeStats(node: TreeNode, selected: ReadonlySet<string>): NodeStats {
  if (node.excluded) return { files: 0, selected: 0, selectedTokens: 0 }
  if (node.kind === 'file') {
    const on = selected.has(node.path)
    return { files: 1, selected: on ? 1 : 0, selectedTokens: on ? (node.tokens ?? 0) : 0 }
  }
  const s: NodeStats = { files: 0, selected: 0, selectedTokens: 0 }
  for (const c of node.children ?? []) {
    const cs = nodeStats(c, selected)
    s.files += cs.files
    s.selected += cs.selected
    s.selectedTokens += cs.selectedTokens
  }
  return s
}

export function checkState(node: TreeNode, selected: ReadonlySet<string>): CheckState {
  if (node.excluded) return 'off'
  const s = nodeStats(node, selected)
  if (s.selected === 0) return 'off'
  return s.selected === s.files ? 'on' : 'mixed'
}

/** All selectable file paths at or below the given nodes. */
export function selectableFiles(nodes: TreeNode[]): string[] {
  const out: string[] = []
  const walk = (n: TreeNode) => {
    if (n.excluded) return
    if (n.kind === 'file') out.push(n.path)
    else n.children?.forEach(walk)
  }
  nodes.forEach(walk)
  return out
}

export function findNode(nodes: TreeNode[], path: string): TreeNode | undefined {
  for (const n of nodes) {
    if (n.path === path) return n
    if (n.children && path.startsWith(n.path + '/')) {
      const hit = findNode(n.children, path)
      if (hit) return hit
    }
  }
  return undefined
}

/** Directories first, then files, each in their original order. */
export function sortNodes(nodes: TreeNode[]): TreeNode[] {
  return [...nodes].sort((a, b) => (a.kind === b.kind ? 0 : a.kind === 'dir' ? -1 : 1))
}

/** Keeps nodes whose name matches the filter, plus their ancestors. */
export function filterTree(nodes: TreeNode[], query: string): TreeNode[] {
  const q = query.trim().toLowerCase()
  if (!q) return nodes
  const out: TreeNode[] = []
  for (const n of nodes) {
    if (n.name.toLowerCase().includes(q)) out.push(n)
    else if (n.children) {
      const kids = filterTree(n.children, q)
      if (kids.length) out.push({ ...n, children: kids })
    }
  }
  return out
}

/** The folder holding the most selected tokens, for the "near limit" hint. */
export function biggestFolder(nodes: TreeNode[], selected: ReadonlySet<string>): { name: string; tokens: number } | null {
  const byFolder = new Map<string, number>()
  const walk = (n: TreeNode, parent: string) => {
    if (n.excluded) return
    if (n.kind === 'file') {
      if (selected.has(n.path)) byFolder.set(parent, (byFolder.get(parent) ?? 0) + (n.tokens ?? 0))
    } else n.children?.forEach((c) => walk(c, n.name))
  }
  nodes.forEach((n) => walk(n, ''))
  let best: { name: string; tokens: number } | null = null
  for (const [name, tokens] of byFolder) if (name && (!best || tokens > best.tokens)) best = { name, tokens }
  return best
}
