import { createResource, For, Show } from 'solid-js'
import { useWails } from '../lib/wails'
import { Draft } from '../types/mail'
import { formatFullDate } from '../lib/format'
import { t } from '../i18n'

interface Props {
  open: boolean
  language: string
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
      <div class="drafts-drawer">
        <div class="panel-header">
          <h2>{t(props.language, 'drafts')}</h2>
          <button onClick={props.onClose} class="icon-button">×</button>
        </div>

        <div class="drawer-list">
          <Show when={drafts.state === 'ready'} fallback={<div class="empty-state">Loading...</div>}>
            <For each={drafts()} fallback={<div class="empty-state">No drafts</div>}>
              {(d) => (
                <div class="draft-row" onClick={() => props.onOpenDraft?.(d)}>
                  <div class="draft-subject">{d.subject || '(no subject)'}</div>
                  <div class="draft-to">{d.to}</div>
                  <div class="draft-date">{formatFullDate(d.updatedAt)}</div>
                </div>
              )}
            </For>
          </Show>
        </div>
      </div>
    </Show>
  )
}
