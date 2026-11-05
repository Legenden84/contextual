import { ApplyDialog } from '@/components/apply/ApplyDialog'
import { Toast } from '@/components/apply/Toast'
import { ChangesPanel } from '@/components/changes/ChangesPanel'
import { ChatThread } from '@/components/chat/ChatThread'
import { Composer } from '@/components/chat/Composer'
import { CodePreview } from '@/components/code/CodePreview'
import { Sidebar } from '@/components/explorer/Sidebar'
import { Tabs } from '@/components/shell/Tabs'
import { useApp } from '@/state/app-context'
import { DiffScreen } from './DiffScreen'

/** Three columns: explorer, chat (or a preview tab), Changes. The diff view replaces the last two. */
export function WorkspaceScreen() {
  const { centerView, activeTab, applyOpen, changes } = useApp()

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1">
      <Sidebar />
      {centerView === 'diff' && changes.length > 0 ? (
        <DiffScreen />
      ) : (
        <>
          <main className="flex min-w-0 flex-1 flex-col bg-surface-100">
            <Tabs />
            {activeTab === 'chat' ? <ChatThread /> : <CodePreview key={activeTab} path={activeTab} />}
            <Composer />
          </main>
          {changes.length > 0 && <ChangesPanel />}
        </>
      )}
      {applyOpen && <ApplyDialog />}
      <Toast />
    </div>
  )
}
