import { createSignal, Show } from 'solid-js'
import { MessageDetail } from '../types/mail'
import { t } from '../i18n'
import { useWails } from '../lib/wails'

interface Props {
  detail: MessageDetail
  language: string
  onReload: () => void
}

export function SafeEmailBody(props: Props) {
  const wails = useWails()
  const [status, setStatus] = createSignal('')

  const setRemoteContent = async (scope: 'message' | 'sender', allowed: boolean) => {
    setStatus('')
    try {
      await wails.SetRemoteContentPreference({ messageId: props.detail.id, scope, allowed })
      props.onReload()
    } catch (error) {
      setStatus(error instanceof Error ? error.message : String(error))
    }
  }

  const handleBodyClick = async (event: MouseEvent) => {
    const target = event.target as HTMLElement | null
    const anchor = target?.closest('a')
    if (!anchor) return
    const href = anchor.getAttribute('href') ?? ''
    event.preventDefault()
    try {
      await wails.OpenExternalLink({ url: href })
    } catch (error) {
      setStatus(error instanceof Error ? error.message : String(error))
    }
  }

  return (
    <div class="safe-email">
      <Show when={props.detail.remoteContentBlocked > 0 || props.detail.unsafeHtmlRemoved}>
        <div class="render-banner">
          <span>{t(props.language, 'safe')}</span>
          <Show when={props.detail.remoteContentBlocked > 0}>
            <span>{props.detail.remoteContentBlocked} {t(props.language, 'remoteBlocked')}</span>
          </Show>
          <Show when={props.detail.unsafeHtmlRemoved}>
            <span>{t(props.language, 'sanitized')}</span>
          </Show>
        </div>
      </Show>
      <Show when={props.detail.remoteContentBlocked > 0}>
        <div class="remote-actions">
          <button class="secondary-button" onClick={() => setRemoteContent('message', true)}>
            {t(props.language, 'loadRemoteOnce')}
          </button>
          <button class="secondary-button" onClick={() => setRemoteContent('sender', true)}>
            {t(props.language, 'loadRemoteSender')}
          </button>
        </div>
      </Show>
      <Show when={props.detail.remoteContentAllowed}>
        <div class="remote-actions">
          <button class="secondary-button" onClick={() => setRemoteContent('message', false)}>
            {t(props.language, 'blockRemote')}
          </button>
        </div>
      </Show>
      <Show when={status()}>
        <div class="panel-status">{status()}</div>
      </Show>
      <div class="safe-email-body" innerHTML={props.detail.safeHtml || '<p></p>'} onClick={handleBodyClick} />
    </div>
  )
}
