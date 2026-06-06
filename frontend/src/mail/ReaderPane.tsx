import { createResource, createSignal, For, Show } from 'solid-js'
import { formatFullDate } from '../lib/format'
import { AttachmentPreview, MessageDetail, MessageRow } from '../types/mail'
import { SafeEmailBody } from './SafeEmailBody'
import { useWails } from '../lib/wails'
import { t } from '../i18n'

interface Props {
  message: MessageRow
  detail?: MessageDetail | null
  language: string
  onReloadDetail: () => void
}

export function ReaderPane(props: Props) {
  const wails = useWails()
  const [preview, setPreview] = createSignal<AttachmentPreview | null>(null)
  const [attachmentStatus, setAttachmentStatus] = createSignal('')
  const [attachments, { refetch: refetchAttachments }] = createResource(
    () => props.detail?.id ?? '',
    async (messageId) => {
      if (!messageId) return []
      return await wails.ListAttachments({ messageId })
    },
  )

  const subject = () => props.detail?.subject || props.message.subject
  const senderName = () => props.detail?.senderName || props.message.senderName
  const senderEmail = () => props.detail?.senderEmail || props.message.senderEmail
  const dateReceived = () => props.detail?.dateReceived || props.message.dateReceived

  const previewAttachment = async (attachmentId: string) => {
    setAttachmentStatus('')
    try {
      setPreview(await wails.PreviewAttachment({ attachmentId }))
    } catch (error) {
      setAttachmentStatus(error instanceof Error ? error.message : String(error))
    }
  }

  const saveAttachment = async (attachmentId: string) => {
    setAttachmentStatus('')
    try {
      const result = await wails.SaveAttachment({ attachmentId })
      setAttachmentStatus(result.savedPath ? `${t(props.language, 'saved')}: ${result.savedPath}` : t(props.language, 'saved'))
    } catch (error) {
      setAttachmentStatus(error instanceof Error ? error.message : String(error))
    }
  }

  const deleteAttachment = async (attachmentId: string) => {
    setAttachmentStatus('')
    try {
      await wails.DeleteAttachment({ attachmentId })
      setPreview(null)
      await refetchAttachments()
    } catch (error) {
      setAttachmentStatus(error instanceof Error ? error.message : String(error))
    }
  }

  return (
    <div class="reader">
      <header class="reader-header">
        <h1>{subject()}</h1>
        <div class="reader-meta">
          <span>{senderName()}</span>
          <span>{senderEmail()}</span>
          <span>{formatFullDate(dateReceived())}</span>
        </div>
      </header>

      <Show when={props.detail} fallback={
        <div class="reader-placeholder">
          <p>{props.message.snippet}</p>
        </div>
      }>
        {(detail) => (
          <>
            <SafeEmailBody detail={detail()} language={props.language} onReload={props.onReloadDetail} />
            <Show when={(attachments() ?? []).length > 0}>
              <section class="attachment-panel">
                <h2>{t(props.language, 'attachments')}</h2>
                <div class="attachment-list">
                  <For each={attachments() ?? []}>
                    {(attachment) => (
                      <div class="attachment-row">
                        <div class="attachment-main">
                          <strong>{attachment.displayName}</strong>
                          <span>{attachment.mime} · {attachment.sizeBytes} B</span>
                        </div>
                        <div class="attachment-actions">
                          <button class="secondary-button" onClick={() => previewAttachment(attachment.id)}>{t(props.language, 'preview')}</button>
                          <button class="secondary-button" onClick={() => saveAttachment(attachment.id)}>{t(props.language, 'saveAttachment')}</button>
                          <button class="danger-button" onClick={() => deleteAttachment(attachment.id)}>{t(props.language, 'deleteAttachment')}</button>
                        </div>
                      </div>
                    )}
                  </For>
                </div>
              </section>
            </Show>
            <Show when={preview()}>
              {(item) => (
                <div class="attachment-preview">
                  <Show when={item().kind === 'text'}>
                    <pre>{item().text}</pre>
                  </Show>
                  <Show when={item().kind === 'image'}>
                    <img src={item().dataUrl} alt={item().displayName} />
                  </Show>
                  <Show when={item().kind === 'unsupported'}>
                    <p>{t(props.language, 'unsupportedPreview')}</p>
                  </Show>
                </div>
              )}
            </Show>
            <Show when={attachmentStatus()}>
              <div class="panel-status">{attachmentStatus()}</div>
            </Show>
          </>
        )}
      </Show>
    </div>
  )
}
