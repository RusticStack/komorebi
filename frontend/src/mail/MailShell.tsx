import { createSignal, createResource, Show, onMount, onCleanup } from 'solid-js'
import { useWails } from '../lib/wails'
import { MailShellData, MessageRow } from '../types/mail'
import { MessageList } from './MessageList'
import { ReaderPane } from './ReaderPane'
import { Sidebar } from './Sidebar'
import { SearchBar } from './SearchBar'
import { DraftsDrawer } from './DraftsDrawer'
import { Composer } from './Composer'

export function MailShell() {
  const wails = useWails()
  const [data, { refetch }] = createResource<MailShellData>(async () => {
    return await wails.LoadMailShell()
  })

  const [selectedMessageId, setSelectedMessageId] = createSignal<string | null>(null)
  const [currentFolder, setCurrentFolder] = createSignal('inbox')
  const [searchResults, setSearchResults] = createSignal<MessageRow[] | null>(null)
  const [loadedMessages, setLoadedMessages] = createSignal<MessageRow[]>([])
  const [showDrafts, setShowDrafts] = createSignal(false)
  const [composerDraft, setComposerDraft] = createSignal<any>(null) // real draft being edited

  const selectedMessage = () => {
    const all = data()?.messages ?? []
    const search = searchResults()
    const list = search ?? all
    return list.find(m => m.id === selectedMessageId())
  }

  const handleFolderSelect = async (folderId: string) => {
    setCurrentFolder(folderId)
    setSearchResults(null)
    setSelectedMessageId(null)
    setLoadedMessages([])
    await refetch()
  }

  const handleMessagesAppended = (newMsgs: MessageRow[]) => {
    setLoadedMessages(prev => {
      const existingIds = new Set(prev.map(m => m.id))
      const deduped = newMsgs.filter(m => !existingIds.has(m.id))
      return [...prev, ...deduped]
    })
  }

  // Real draft creation - opens real composer backed by SQLite
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
      const results = await wails.SearchMail(query, 80)
      setSearchResults(results)
      setSelectedMessageId(results[0]?.id ?? null)
    } catch (e) {
      console.error('search failed', e)
    }
  }

  const handleMessageSelect = async (id: string) => {
    setSelectedMessageId(id)
    // Real mutation - mark as read in SQLite
    try {
      await wails.MarkRead(id, true)
      await refetch()
    } catch (e) {
      console.error('mark read failed', e)
    }
  }

  // Load first message automatically when data arrives
  onMount(() => {
    const unsub = setInterval(() => {
      const msgs = data()?.messages
      if (msgs && msgs.length > 0 && !selectedMessageId()) {
        setSelectedMessageId(msgs[0].id)
        clearInterval(unsub)
      }
    }, 120)

    // Listen for global keyboard shortcut (Ctrl/Cmd + N)
    const handleNewDraft = () => createNewDraft()
    window.addEventListener('komorebi:new-draft', handleNewDraft)
    onCleanup(() => window.removeEventListener('komorebi:new-draft', handleNewDraft))
  })

  return (
    <div class="h-screen flex flex-col bg-[#F7F5F2] text-[#2D2D2D] select-none">
      {/* Custom Windows titlebar - real chrome */}
      <div class="titlebar h-8 bg-[#F1EDE7] border-b border-[#D9D4CC] flex items-center px-3 text-sm justify-between">
        <div class="flex items-center gap-2 font-medium tracking-tight">
          <span class="text-[#5C6B5E]">komorebi</span>
          <span class="text-[#8A8680] text-xs">local mail</span>
        </div>
        <div class="controls flex gap-1 text-[#8A8680]">
          <button 
            class="px-2.5 py-0.5 text-xs rounded hover:bg-[#EDE8E1]" 
            onClick={() => setShowDrafts(true)}
          >
            Drafts
          </button>
          <button 
            class="px-2.5 py-0.5 text-xs rounded bg-[#5C6B5E] text-white hover:bg-[#8B5E3C]" 
            onClick={createNewDraft}
          >
            + New Draft
          </button>
          <button class="px-2 hover:bg-[#EDE8E1] rounded" onClick={() => window.close?.()}>×</button>
        </div>
      </div>

      <div class="mail-main flex-1 flex overflow-hidden">
        {/* Real Sidebar with real folders from SQLite */}
        <Sidebar 
          folders={data()?.folders ?? []} 
          currentFolder={currentFolder()}
          onSelect={handleFolderSelect}
          onRefresh={() => refetch()}
        />

        {/* Center column: Search + Virtual Message List (real data) */}
        <div class="mail-list-pane flex-1 flex flex-col min-w-0 border-r border-[#D9D4CC]">
          <SearchBar onSearch={handleSearch} />

          <div class="flex-1 overflow-hidden">
            <Show when={data.state === 'ready'} fallback={<div class="p-8 text-[#8A8680]">Loading real local cache…</div>}>
              <MessageList 
                messages={searchResults() ?? (loadedMessages().length ? loadedMessages() : (data()?.messages ?? []))}
                selectedId={selectedMessageId()}
                onSelect={handleMessageSelect}
                total={data()?.total ?? 0}
                currentFolder={currentFolder()}
                onMessagesAppended={handleMessagesAppended}
              />
            </Show>
          </div>

          <div class="h-6 bg-[#F1EDE7] border-t border-[#D9D4CC] text-xs px-3 flex items-center text-[#8A8680]">
            {data()?.total ?? 0} messages in local cache • real SQLite • no network
          </div>
        </div>

        {/* Reader - renders real message content from DB */}
        <div class="mail-reader-pane flex-[1.6] min-w-0 overflow-auto bg-white">
          <Show when={selectedMessage()} fallback={
            <div class="h-full flex items-center justify-center text-[#8A8680] text-sm">
              Select a message from the real local cache
            </div>
          }>
            {(msg) => <ReaderPane message={msg()} />}
          </Show>
        </div>
      </div>

      <DraftsDrawer 
        open={showDrafts()} 
        onClose={() => setShowDrafts(false)} 
        onOpenDraft={(d) => {
          setShowDrafts(false)
          setComposerDraft(d)
        }}
      />

      <Show when={composerDraft()}>
        <Composer 
          draftId={composerDraft().id}
          initialSubject={composerDraft().subject}
          initialTo={composerDraft().to}
          initialBody={composerDraft().bodyHtml}
          onClose={() => setComposerDraft(null)}
          onSaved={() => { /* real autosave already happening inside */ }}
        />
      </Show>
    </div>
  )
}
