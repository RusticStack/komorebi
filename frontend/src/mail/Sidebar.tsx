import { createResource, For, Show } from 'solid-js'
import { FolderView } from '../types/mail'
import { useWails } from '../lib/wails'

interface Props {
  folders: FolderView[]
  currentFolder: string
  onSelect: (id: string) => void
  onRefresh: () => void
}

export function Sidebar(props: Props) {
  const wails = useWails()
  const [accounts] = createResource(() => wails.ListAccounts())

  return (
    <div class="mail-sidebar w-56 bg-[#EDE8E1] border-r border-[#D9D4CC] flex flex-col">
      <div class="p-3 text-xs uppercase tracking-widest text-[#8A8680] font-medium">
        Accounts (real)
      </div>
      <div class="px-2 text-xs">
        <For each={accounts() || []}>
          {(acc) => (
            <div class="px-2 py-1 cursor-pointer hover:bg-[#F7F5F2]" onClick={() => window.location.reload()}>
              {acc.displayName || acc.email}
            </div>
          )}
        </For>
      </div>

      <div class="p-3 text-xs uppercase tracking-widest text-[#8A8680] font-medium flex justify-between items-center">
        <span>Folders</span>
        <button onClick={props.onRefresh} class="text-[#5C6B5E] hover:text-[#8B5E3C]">↻</button>
      </div>

      <div class="px-2 py-1 text-sm">
        <For each={props.folders}>
          {(folder) => (
            <div
              class={`folder-item ${props.currentFolder === folder.id ? 'active' : ''}`}
              onClick={() => props.onSelect(folder.id)}
            >
              <span>{folder.displayName || folder.name}</span>
              <Show when={folder.unreadCount > 0}>
                <span class="text-xs px-1.5 py-px rounded bg-[#8B5E3C] text-white">{folder.unreadCount}</span>
              </Show>
            </div>
          )}
        </For>
      </div>

      <div class="mt-auto p-3 text-[10px] text-[#8A8680] border-t border-[#D9D4CC]">
        Real SQLite cache<br />
        <button class="underline mr-2" onClick={async () => {
          const email = prompt('Email for new real account?')
          if (email) {
            const pw = prompt('Password (stored ONLY in Windows Credential Manager via wincred)')
            await (window as any).wails?.AddAccount?.(email, email.split('@')[0], 'generic', pw || '')
            window.location.reload()
          }
        }}>Add real account</button>
        <button class="underline" onClick={async () => {
          await (window as any).wails?.SyncAccount?.('demo-local')
          window.location.reload()
        }}>Sync Now (real IMAP)</button>
      </div>
    </div>
  )
}
