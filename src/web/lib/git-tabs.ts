/** Center tabs for Source Control diffs: `git:staged:<path>` (HEAD → index) or `git:changes:<path>` (index → file). */
export const gitTabId = (path: string, staged: boolean) => `git:${staged ? 'staged' : 'changes'}:${path}`

export function parseGitTab(tab: string): { staged: boolean; path: string } | null {
  const m = /^git:(staged|changes):(.+)$/.exec(tab)
  return m ? { staged: m[1] === 'staged', path: m[2] } : null
}
