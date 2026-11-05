import { StatusBar } from '@/components/shell/StatusBar'
import { TitleBar } from '@/components/shell/TitleBar'
import { FolderDialog } from '@/components/project/FolderDialog'
import { SettingsScreen } from '@/screens/SettingsScreen'
import { WelcomeScreen } from '@/screens/WelcomeScreen'
import { WorkspaceScreen } from '@/screens/WorkspaceScreen'
import { AppProvider } from '@/state/AppProvider'
import { useApp } from '@/state/app-context'

function Shell() {
  const { screen, project, folderDialogOpen } = useApp()
  const title =
    screen === 'settings' ? 'Settings — Contextual' : screen === 'workspace' && project ? `${project.name} — Contextual` : 'Contextual'
  return (
    <div className="flex h-svh flex-col overflow-hidden bg-surface-100 text-ui-body text-ink">
      <TitleBar title={title} />
      <div className="relative flex min-h-0 flex-1">
        {screen === 'welcome' && <WelcomeScreen />}
        {screen === 'workspace' && project && <WorkspaceScreen />}
        {screen === 'settings' && <SettingsScreen />}
        {folderDialogOpen && <FolderDialog />}
      </div>
      <StatusBar />
    </div>
  )
}

export default function App() {
  return (
    <AppProvider>
      <Shell />
    </AppProvider>
  )
}
