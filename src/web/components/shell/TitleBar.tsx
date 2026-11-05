import { IconButton } from '@/components/ui/Button'
import { useApp } from '@/state/app-context'
import { Logo } from './Logo'

export function TitleBar({ title }: { title: string }) {
  const { theme, toggleTheme, screen, closeProject } = useApp()
  return (
    <header className="relative flex h-[34px] flex-none items-center gap-3 border-b border-border bg-surface-000 pr-2 pl-3 text-ui-small">
      <Logo />
      <nav className="flex gap-0.5 text-ink-muted">
        <button
          type="button"
          className="cursor-pointer rounded-sm px-2 py-0.5 hover:bg-surface-hover hover:text-ink"
          onClick={closeProject}
          disabled={screen === 'welcome'}
        >
          File
        </button>
        <span className="px-2 py-0.5">Edit</span>
        <span className="px-2 py-0.5">View</span>
        <span className="px-2 py-0.5">Help</span>
      </nav>
      <span className="absolute left-1/2 -translate-x-1/2 text-ink-muted">{title}</span>
      <span className="flex-1" />
      <IconButton
        icon={theme === 'dark' ? 'sun' : 'moon'}
        label={theme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme'}
        onClick={toggleTheme}
      />
    </header>
  )
}
