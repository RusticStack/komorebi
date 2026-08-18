import { createVirtualizer } from '@tanstack/solid-virtual'
import { For, Show, createEffect, createSignal, onCleanup, onMount } from 'solid-js'
import { MessageRow } from '../types/mail'
import { useWails } from '../lib/wails'
import { formatRelativeDate } from '../lib/format'
import { t } from '../i18n'

interface Props {
  messages: MessageRow[]
  selectedId: string | null
  onSelect: (id: string) => void
  currentFolder: string
  hasMore: boolean
  onMessagesAppended: (newMessages: MessageRow[], hasMore: boolean) => void
  language: string
}

export function MessageList(props: Props) {
  let containerRef: HTMLDivElement | undefined
  let lastFolder = props.currentFolder
  const wails = useWails()
  const [isLoadingMore, setIsLoadingMore] = createSignal(false)
  const [cursor, setCursor] = createSignal(0)

  const virtualizer = createVirtualizer({
    count: props.messages.length,
    getScrollElement: () => containerRef ?? null,
    estimateSize: () => 66,
    overscan: 10,
  })

  createEffect(() => {
    if (props.currentFolder !== lastFolder) {
      lastFolder = props.currentFolder
      setCursor(props.messages.length)
      containerRef?.scrollTo({ top: 0 })
    }
  })

  createEffect(() => {
    if (cursor() === 0 && props.messages.length > 0) {
      setCursor(props.messages.length)
    }
  })

  const loadMore = async () => {
    if (isLoadingMore() || !props.hasMore) return

    setIsLoadingMore(true)
    try {
      const result = await wails.LoadMessagePage({ folderId: props.currentFolder, cursor: cursor(), limit: 60 })
      if (result?.messages?.length) {
        props.onMessagesAppended(result.messages, result.hasMore)
        setCursor(result.nextCursor || (cursor() + result.messages.length))
      } else {
        props.onMessagesAppended([], false)
      }
    } catch (e) {
      console.error('Failed to load messages', e)
    } finally {
      setIsLoadingMore(false)
    }
  }

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
      }
    }
    window.addEventListener('keydown', handleKey)
    onCleanup(() => window.removeEventListener('keydown', handleKey))
  })

  return (
    <div ref={containerRef} class="message-list">
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
                class={`message-row ${props.selectedId === msg.id ? 'selected' : ''} ${!msg.isRead ? 'unread' : ''}`}
                style={{ top: `${virtualItem.start}px`, height: `${virtualItem.size}px` }}
                onClick={() => props.onSelect(msg.id)}
              >
                <div class="message-row-top">
                  <div class="message-sender">{msg.senderName || msg.senderEmail}</div>
                  <div class="message-date">{formatRelativeDate(msg.dateReceived)}</div>
                </div>
                <div class="message-subject">{msg.subject}</div>
                <div class="message-snippet">{msg.snippet}</div>
              </div>
            )
          }}
        </For>
      </div>

      <Show when={isLoadingMore()}>
        <div class="list-status">Loading...</div>
      </Show>

      <Show when={props.messages.length === 0}>
        <div class="empty-state">{t(props.language, 'emptyFolder')}</div>
      </Show>
    </div>
  )
}
