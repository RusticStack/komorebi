import { For, Show } from 'solid-js'
import { AccountView, BackgroundSyncStatus, FolderView } from '../types/mail'
import { t } from '../i18n'

interface Props {
  accounts: AccountView[]
  folders: FolderView[]
  currentFolder: string
  language: string
  onSelect: (id: string) => void
  onRefresh: () => void
  onOpenSetup: () => void
  onSyncAccount: (id: string) => void
  syncStatuses: BackgroundSyncStatus[]
  onStartAutoSync: (id: string) => void
  onStopAutoSync: (id: string) => void
}

export function Sidebar(props: Props) {
  const syncStatusFor = (accountId: string) => props.syncStatuses.find((item) => item.accountId === accountId)

  return (
    <aside class="mail-sidebar">
      <div class="sidebar-section-header">
        <span>{t(props.language, 'accounts')}</span>
        <button class="icon-button" title={t(props.language, 'addAccount')} onClick={props.onOpenSetup}>+</button>
      </div>

      <div class="account-list">
        <For each={props.accounts}>
          {(account) => {
            const syncStatus = () => syncStatusFor(account.id)
            return (
              <div class="account-row">
                <div class="account-main">
                  <span class="account-name">{account.displayName || account.email}</span>
                  <span class={`account-safety ${account.safety}`}>{account.authMode}</span>
                  <Show when={syncStatus()?.lastError}>
                    <span class="account-sync-error">{syncStatus()?.lastError}</span>
                  </Show>
                  <Show when={syncStatus()?.running && syncStatus()?.lastRun}>
                    <span class="account-sync-note">{t(props.language, 'lastSync')} {new Date((syncStatus()?.lastRun ?? 0) * 1000).toLocaleTimeString()}</span>
                  </Show>
                </div>
                <Show when={account.provider !== 'local'}>
                  <div class="account-actions">
                    <button class="ghost-button" onClick={() => props.onSyncAccount(account.id)}>
                      {t(props.language, 'sync')}
                    </button>
                    <Show
                      when={syncStatus()?.running}
                      fallback={
                        <button class="ghost-button" onClick={() => props.onStartAutoSync(account.id)}>
                          {t(props.language, 'autoSync')}
                        </button>
                      }
                    >
                      <button class="ghost-button active" onClick={() => props.onStopAutoSync(account.id)}>
                        {t(props.language, 'stopSync')}
                      </button>
                    </Show>
                  </div>
                </Show>
              </div>
            )
          }}
        </For>
      </div>

      <div class="sidebar-section-header">
        <span>{t(props.language, 'folders')}</span>
        <button class="icon-button" title="Refresh" onClick={props.onRefresh}>↻</button>
      </div>

      <nav class="folder-list">
        <For each={props.folders}>
          {(folder) => (
            <button
              class={`folder-item ${props.currentFolder === folder.id ? 'active' : ''}`}
              onClick={() => props.onSelect(folder.id)}
            >
              <span>{folder.displayName || folder.name}</span>
              <Show when={folder.unreadCount > 0}>
                <span class="folder-count">{folder.unreadCount}</span>
              </Show>
            </button>
          )}
        </For>
      </nav>
    </aside>
  )
}
