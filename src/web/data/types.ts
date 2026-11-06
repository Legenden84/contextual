// Shared data types for the explorer, rules, deployments and proposed changes.

export type ChangeStatus = 'A' | 'M' | 'D'

export interface TreeNode {
  name: string
  path: string
  kind: 'dir' | 'file'
  tokens?: number
  children?: TreeNode[]
  /** Exclusion reason (`ignored: node_modules`, `lock file`, `binary`, `.gitignore`). */
  excluded?: string
  /** Changed on disk since it was loaded into context. */
  changedOnDisk?: boolean
  /** Last modified time from the scan, for real folders. */
  mtimeMs?: number
  /** Sent to the model as a picture. */
  image?: boolean
  /** Large HTML/SVG sent condensed: `tokens` is the condensed size, this the original. */
  condensedFrom?: number
}

export interface Rule {
  path: string
  type: 'Rule' | 'Skill'
  tokens: number
  content: string
}

export interface Deployment {
  name: string
  limit: number
  /** USD per 1M input tokens. */
  inPrice: number
  /** USD per 1M output tokens. */
  outPrice: number
}

export interface Change {
  path: string
  status: ChangeStatus
  old: string
  new: string
  /** Set when the file changed on disk after it was sent to the model. */
  conflict?: string
  /** Internal: the edit blocks this change was built from (existing files). */
  edits?: { search: string; replace: string }[]
  /** Internal: edit blocks that couldn't be placed in the file; resolved before the change is shown. */
  unplaced?: { search: string; replace: string }[]
}
