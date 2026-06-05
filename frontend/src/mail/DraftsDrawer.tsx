import { createResource, For, Show } from 'solid-js'
import { useWails } from '../lib/wails'
import { Draft } from '../types/mail'
import { formatFullDate } from '../lib/format'

interface Props {
  open: boolean
  onClose: () => void
  onOpenDraft?: (draft: any) => void
}

export function DraftsDrawer(props: Props) {
  const wails = useWails()
  const [drafts] = createResource<Draft[]>(async () => {
    return await wails.ListDrafts()
  })

  return (
    <Show when={props.open}>
      <div class="fixed right-0 top-8 bottom-0 w-96 bg-white border-l border-[#D9D4CC] shadow-xl z-50 flex flex-col">
        <div class="p-4 border-b flex justify-between items-center">
          <div class="font-semibold">Drafts (real local SQLite)</div>
          <button onClick={props.onClose} class="text-xl leading-none">×</button>
        </div>

        <div class="flex-1 overflow-auto p-2 text-sm">
          <Show when={drafts.state === 'ready'} fallback={<div class="p-4 text-[#8A8680]">Loading real drafts…</div>}>
            <For each={drafts()} fallback={<div class="p-4 text-[#8A8680]">No drafts yet. They will be real persisted rows.</div>}>
              {(d) => (
                <div class="p-3 border-b hover:bg-[#F7F5F2] cursor-pointer" onClick={() => props.onOpenDraft?.(d)}>
                  <div class="font-medium truncate">{d.subject || '(no subject)'}</div>
                  <div class="text-xs text-[#8A8680]">{d.to}</div>
                  <div class="text-[10px] text-[#8A8680] mt-1">{formatFullDate(d.updatedAt)}</div>
                </div>
              )}
            </For>
          </Show>
        </div>

        <div class="p-3 border-t text-xs text-[#8A8680]">
          Drafts are stored locally only (same model as the original design).
        </div>
      </div>
    </Show>
  )
}
