import { langFromPath } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import { Icon } from './Icon'

const GLYPH: Partial<Record<ReturnType<typeof langFromPath>, [string, string]>> = {
  csharp: ['C#', 'text-file-csharp'],
  json: ['{ }', 'text-file-json'],
  xml: ['</>', 'text-file-xml'],
  markdown: ['M↓', 'text-file-markdown'],
  yaml: ['!', 'text-file-xml'],
  typescript: ['TS', 'text-file-markdown'],
  python: ['py', 'text-file-markdown'],
  shell: ['$', 'text-ink-muted'],
  sql: ['db', 'text-file-json'],
}

const box = 'inline-flex w-4 flex-none items-center justify-center font-mono text-[9px] font-semibold tracking-[-0.02em]'

/** Small coloured file-type glyph used in trees, tabs and headers. */
export function FileGlyph({ name, className }: { name: string; className?: string }) {
  if (/\.sln$/.test(name)) return <span className={cn(box, 'text-file-xml', className)}>sln</span>
  if (/\.(png|jpg|jpeg|gif|ico|dll|exe|pdb)$/i.test(name)) return <FileIcon className={className} />
  const g = GLYPH[langFromPath(name)]
  if (!g) return <FileIcon className={className} />
  return <span className={cn(box, g[1], className)}>{g[0]}</span>
}

function FileIcon({ className }: { className?: string }) {
  return (
    <span className={cn(box, 'text-ink-muted', className)}>
      <Icon name="file" className="size-[15px]" />
    </span>
  )
}

export function FolderGlyph({ open, className }: { open: boolean; className?: string }) {
  return (
    <span className={cn(box, 'text-file-folder', className)}>
      <Icon name={open ? 'folderOpen' : 'folder'} className="size-[15px]" />
    </span>
  )
}
