import { createVirtualizer } from '@tanstack/solid-virtual'
import { For, Show, createSignal, createEffect, onCleanup, onMount } from 'solid-js'
import { MessageRow } from '../types/mail'
import { useWails } from '../lib/wails'
import { formatRelativeDate } from '../lib/format'

interface Props {
  messages: MessageRow[]
  selectedId: string | null
  onSelect: (id: string) => void
  total: number
  currentFolder: string
  onMessagesAppended: (newMessages: MessageRow[]) => void
}

export function MessageList(props: Props) {
  let containerRef: HTMLDivElement | undefined
  const wails = useWails()
  const [isLoadingMore, setIsLoadingMore] = createSignal(false)
  const [cursor, setCursor] = createSignal(0)

  const virtualizer = createVirtualizer({
    count: props.messages.length,
    getScrollElement: () => containerRef ?? null,
    estimateSize: () => 58,
    overscan: 10,
  })

  // Real infinite scroll using the Go LoadMessagePage binding
  const loadMore = async () => {
    if (isLoadingMore() || props.messages.length >= props.total) return

    setIsLoadingMore(true)
    try {
      const result = await wails.LoadMessagePage(props.currentFolder, cursor(), 60)
      if (result?.messages?.length) {
        props.onMessagesAppended(result.messages)
        setCursor(result.nextCursor || (cursor() + result.messages.length))
      }
    } catch (e) {
      console.error('Failed to load more real messages', e)
    } finally {
      setIsLoadingMore(false)
    }
  }

  // Trigger loadMore when user scrolls near the bottom of the virtual list
  createEffect(() => {
    const el = containerRef
    if (!el) return

    const handler = () => {
      const { scrollTop, scrollHeight, clientHeight } = el
      if (scrollHeight - scrollTop - clientHeight < 300) {
        loadMore()
      }
    }

    el.addEventListener('scroll', handler, { passive: true })
    onCleanup(() => el.removeEventListener('scroll', handler))
  })

  // Real keyboard navigation (arrows + enter)
  onMount(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (!props.messages.length) return
      const currentIdx = props.messages.findIndex(m => m.id === props.selectedId)
      if (e.key === 'ArrowDown') {
        const next = Math.min(currentIdx + 1, props.messages.length - 1)
        props.onSelect(props.messages[next].id)
        e.preventDefault()
      } else if (e.key === 'ArrowUp') {
        const prev = Math.max(currentIdx - 1, 0)
        props.onSelect(props.messages[prev].id)
        e.preventDefault()
      } else if (e.key === 'Enter' && props.selectedId) {
        // Could trigger reader focus or reply in future
        e.preventDefault()
      }
    }
    window.addEventListener('keydown', handleKey)
    onCleanup(() => window.removeEventListener('keydown', handleKey))
  })

  return (
    <div ref={containerRef} class="message-list h-full overflow-auto">
      <div
        style={{
          height: `${virtualizer.getTotalSize()}px`,
          width: '100%',
          position: 'relative',
        }}
      >
        <For each={virtualizer.getVirtualItems()}>
          {(virtualItem) => {
            const msg = props.messages[virtualItem.index]
            if (!msg) return null

            return (
              <div
                class={`message-row absolute left-0 right-0 ${props.selectedId === msg.id ? 'selected' : ''} ${!msg.isRead ? 'unread' : ''}`}
                style={{ top: `${virtualItem.start}px`, height: `${virtualItem.size}px` }}
                onClick={() => props.onSelect(msg.id)}
              >
                <div class="flex justify-between text-sm">
                  <div class="font-medium truncate pr-3">{msg.senderName || msg.senderEmail}</div>
                  <div class="text-xs text-[#8A8680] shrink-0">
                    {formatRelativeDate(msg.dateReceived)}
                  </div>
                </div>
                <div class="subject text-[13px] truncate">{msg.subject}</div>
                <div class="text-xs text-[#8A8680] line-clamp-1 mt-0.5">{msg.snippet}</div>
              </div>
            )
          }}
        </For>
      </div>

      <Show when={isLoadingMore()}>
        <div class="p-3 text-center text-xs text-[#8A8680]">Loading more from real SQLite…</div>
      </Show>

      <Show when={props.messages.length === 0}>
        <div class="p-8 text-center text-[#8A8680] text-sm">No messages in this real local folder yet</div>
      </Show>
    </div>
  )
}
