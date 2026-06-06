import { createEffect, createResource, createSignal, onCleanup, onMount, Show } from 'solid-js'
import { useWails } from '../lib/wails'
import { BackgroundSyncStatus, MailShellData, MessageRow, UIPreferences } from '../types/mail'
import { t } from '../i18n'
import { MessageList } from './MessageList'
import { ReaderPane } from './ReaderPane'
import { Sidebar } from './Sidebar'
import { SearchBar } from './SearchBar'
import { DraftsDrawer } from './DraftsDrawer'
import { Composer } from './Composer'
import { AccountSetupPanel } from './AccountSetupPanel'
import { DiagnosticsPanel } from './DiagnosticsPanel'

export function MailShell() {
  const wails = useWails()
  const [data, { refetch }] = createResource<MailShellData>(() => wails.LoadMailShell())
  const [policies] = createResource(() => wails.ListProviderPolicies())
  const [health, { refetch: refetchHealth }] = createResource(() => wails.GetDiagnosticsHealth())
  const [preferences, { mutate: setPreferences }] = createResource<UIPreferences>(() => wails.GetUIPreferences())

  const [selectedMessageId, setSelectedMessageId] = createSignal<string | null>(null)
  const [currentFolder, setCurrentFolder] = createSignal('inbox')
  const [searchResults, setSearchResults] = createSignal<MessageRow[] | null>(null)
  const [loadedMessages, setLoadedMessages] = createSignal<MessageRow[]>([])
  const [folderHasMore, setFolderHasMore] = createSignal(true)
  const [showDrafts, setShowDrafts] = createSignal(false)
  const [showSetup, setShowSetup] = createSignal(false)
  const [showDiagnostics, setShowDiagnostics] = createSignal(false)
  const [composerDraft, setComposerDraft] = createSignal<any>(null)
  const [syncStatus, setSyncStatus] = createSignal('')
  const [backgroundStatuses, setBackgroundStatuses] = createSignal<BackgroundSyncStatus[]>([])

  const language = () => preferences()?.language ?? 'en'
  const density = () => preferences()?.density ?? 'comfortable'

  const messages = () => searchResults() ?? (loadedMessages().length ? loadedMessages() : (data()?.messages ?? []))
  const selectedMessage = () => messages().find(m => m.id === selectedMessageId()) ?? null
  const [messageDetail, { refetch: refetchMessageDetail }] = createResource(selectedMessageId, async (id) => {
    if (!id) return null
    return await wails.LoadMessageDetail({ messageId: id })
  })

  createEffect(() => {
    const shell = data()
    if (!shell) return
    if (!currentFolder() && shell.folders[0]) {
      setCurrentFolder(shell.folders[0].id)
    }
    if (!selectedMessageId() && shell.messages[0]) {
      setSelectedMessageId(shell.messages[0].id)
    }
    setFolderHasMore(shell.hasMore)
  })

  const handleFolderSelect = async (folderId: string) => {
    setCurrentFolder(folderId)
    setSearchResults(null)
    setSelectedMessageId(null)
    setLoadedMessages([])
    const page = await wails.LoadMessagePage({ folderId, cursor: 0, limit: 60 })
    setLoadedMessages(page.messages ?? [])
    setFolderHasMore(page.hasMore)
    setSelectedMessageId(page.messages?.[0]?.id ?? null)
  }

  const handleMessagesAppended = (newMsgs: MessageRow[], hasMore: boolean) => {
    setLoadedMessages(prev => {
      const existingIds = new Set(prev.map(m => m.id))
      const deduped = newMsgs.filter(m => !existingIds.has(m.id))
      return [...prev, ...deduped]
    })
    setFolderHasMore(hasMore)
  }

  const createNewDraft = () => {
    setComposerDraft({
      id: 'draft-' + Date.now(),
      subject: '',
      to: '',
      bodyHtml: '<p></p>',
    })
  }

  const handleSearch = async (query: string) => {
    if (!query.trim()) {
      setSearchResults(null)
      return
    }
    try {
      const results = await wails.SearchMail({ query, limit: 80 })
      setSearchResults(results)
      setSelectedMessageId(results[0]?.id ?? null)
    } catch (e) {
      console.error('search failed', e)
    }
  }

  const handleMessageSelect = async (id: string) => {
    setSelectedMessageId(id)
    try {
      await wails.MarkRead({ messageId: id, read: true })
    } catch (e) {
      console.error('mark read failed', e)
    }
  }

  const handleSyncAccount = async (accountId: string) => {
    setSyncStatus('')
    try {
      await wails.SyncAccount({ accountId })
      await refetch()
      await refetchHealth()
      await refreshBackgroundStatuses()
      setSyncStatus('Synced')
    } catch (e) {
      setSyncStatus(e instanceof Error ? e.message : String(e))
    }
  }

  const refreshBackgroundStatuses = async () => {
    try {
      setBackgroundStatuses(await wails.GetBackgroundSyncStatus())
    } catch (e) {
      console.error('background sync status failed', e)
    }
  }

  const handleStartBackgroundSync = async (accountId: string) => {
    setSyncStatus('')
    try {
      const status = await wails.StartBackgroundSync({ accountId, intervalSeconds: 300, folderPolicy: 'special' })
      setBackgroundStatuses(prev => [...prev.filter(item => item.accountId !== accountId), status])
      setSyncStatus(t(language(), 'autoSyncStarted'))
      window.setTimeout(() => {
        void refreshBackgroundStatuses()
        void refetch()
        void refetchHealth()
      }, 1200)
    } catch (e) {
      setSyncStatus(e instanceof Error ? e.message : String(e))
    }
  }

  const handleStopBackgroundSync = async (accountId: string) => {
    setSyncStatus('')
    try {
      await wails.StopBackgroundSync({ accountId })
      setBackgroundStatuses(prev => prev.filter(item => item.accountId !== accountId))
      setSyncStatus(t(language(), 'autoSyncStopped'))
    } catch (e) {
      setSyncStatus(e instanceof Error ? e.message : String(e))
    }
  }

  const updatePreferences = async (next: Partial<UIPreferences>) => {
    const updated = await wails.UpdateUIPreferences({
      language: next.language ?? language(),
      density: next.density ?? density(),
    })
    setPreferences(updated)
  }

  onMount(() => {
    const handleNewDraft = () => createNewDraft()
    window.addEventListener('komorebi:new-draft', handleNewDraft)
    void refreshBackgroundStatuses()
    const statusTimer = window.setInterval(() => {
      void refreshBackgroundStatuses()
    }, 15000)
    onCleanup(() => {
      window.removeEventListener('komorebi:new-draft', handleNewDraft)
      window.clearInterval(statusTimer)
    })
  })

  return (
    <div class={`mail-app density-${density()}`}>
      <div class="titlebar">
        <div class="title-group">
          <span class="app-mark">{t(language(), 'appName')}</span>
          <span>{t(language(), 'localMail')}</span>
        </div>
        <div class="title-actions">
          <button class="toolbar-button" onClick={() => setShowDiagnostics(true)}>{t(language(), 'diagnostics')}</button>
          <button class="toolbar-button" onClick={() => setShowDrafts(true)}>{t(language(), 'drafts')}</button>
          <button class="primary-button small" onClick={createNewDraft}>{t(language(), 'newDraft')}</button>
          <select
            class="mini-select"
            value={language()}
            onChange={(e) => updatePreferences({ language: e.currentTarget.value as UIPreferences['language'] })}
          >
            <option value="en">EN</option>
            <option value="pt-PT">PT</option>
          </select>
          <button
            class="icon-button"
            title={density() === 'compact' ? t(language(), 'comfortable') : t(language(), 'compact')}
            onClick={() => updatePreferences({ density: density() === 'compact' ? 'comfortable' : 'compact' })}
          >
            {density() === 'compact' ? '↕' : '↔'}
          </button>
          <button class="icon-button" onClick={() => window.close?.()}>×</button>
        </div>
      </div>

      <div class="mail-main">
        <Sidebar
          accounts={data()?.accounts ?? []}
          folders={data()?.folders ?? []}
          currentFolder={currentFolder()}
          language={language()}
          onSelect={handleFolderSelect}
          onRefresh={async () => {
            await refetch()
            await refetchHealth()
          }}
          onOpenSetup={() => setShowSetup(true)}
          onSyncAccount={handleSyncAccount}
          syncStatuses={backgroundStatuses()}
          onStartAutoSync={handleStartBackgroundSync}
          onStopAutoSync={handleStopBackgroundSync}
        />

        <section class="mail-list-pane">
          <SearchBar onSearch={handleSearch} language={language()} />
          <Show when={syncStatus()}>
            <div class="sync-status">{syncStatus()}</div>
          </Show>

          <div class="list-frame">
            <Show when={data.state === 'ready'} fallback={<div class="empty-state">{t(language(), 'loading')}</div>}>
              <MessageList
                messages={messages()}
                selectedId={selectedMessageId()}
                onSelect={handleMessageSelect}
                currentFolder={currentFolder()}
                hasMore={searchResults() ? false : folderHasMore()}
                onMessagesAppended={handleMessagesAppended}
                language={language()}
              />
            </Show>
          </div>

          <div class="status-bar">
            <span>{messages().length} / {data()?.total ?? messages().length}</span>
            <span>{health()?.schemaOk ? t(language(), 'safe') : t(language(), 'blocked')}</span>
          </div>
        </section>

        <section class="mail-reader-pane">
          <Show when={selectedMessage()} fallback={
            <div class="reader-empty">{t(language(), 'selectMessage')}</div>
          }>
            {(msg) => <ReaderPane message={msg()} detail={messageDetail() ?? null} language={language()} onReloadDetail={() => { void refetchMessageDetail() }} />}
          </Show>
        </section>
      </div>

      <DraftsDrawer
        open={showDrafts()}
        language={language()}
        onClose={() => setShowDrafts(false)}
        onOpenDraft={(d) => {
          setShowDrafts(false)
          setComposerDraft(d)
        }}
      />

      <AccountSetupPanel
        open={showSetup()}
        policies={policies() ?? []}
        language={language()}
        onClose={() => setShowSetup(false)}
        onChanged={async () => {
          await refetch()
          await refetchHealth()
        }}
      />

      <DiagnosticsPanel
        open={showDiagnostics()}
        health={health()}
        language={language()}
        onClose={() => setShowDiagnostics(false)}
      />

      <Show when={composerDraft()}>
        <Composer
          draftId={composerDraft().id}
          initialSubject={composerDraft().subject}
          initialTo={composerDraft().to}
          initialBody={composerDraft().bodyHtml}
          language={language()}
          onClose={() => setComposerDraft(null)}
          onSaved={() => {}}
        />
      </Show>
    </div>
  )
}
