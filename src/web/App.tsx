import { StatusBar } from '@/components/shell/StatusBar'
import { TitleBar } from '@/components/shell/TitleBar'
import { project } from '@/data/sample'
import { SettingsScreen } from '@/screens/SettingsScreen'
import { WelcomeScreen } from '@/screens/WelcomeScreen'
import { WorkspaceScreen } from '@/screens/WorkspaceScreen'
import { AppProvider } from '@/state/AppProvider'
import { useApp } from '@/state/app-context'

const TITLES = {
  welcome: 'Contextual',
  workspace: `${project.name} — Contextual`,
  settings: 'Settings — Contextual',
} as const

function Shell() {
  const { screen } = useApp()
  return (
    <div className="flex h-svh flex-col overflow-hidden bg-surface-100 text-ui-body text-ink">
      <TitleBar title={TITLES[screen]} />
      <div className="relative flex min-h-0 flex-1">
        {screen === 'welcome' && <WelcomeScreen />}
        {screen === 'workspace' && <WorkspaceScreen />}
        {screen === 'settings' && <SettingsScreen />}
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
