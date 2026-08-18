import { createSignal, onCleanup } from 'solid-js'
import { Dialog } from '@ark-ui/solid/dialog'
import { useWails } from '../lib/wails'
import { t } from '../i18n'

interface Props {
  accountId?: string
  draftId?: string
  initialSubject?: string
  initialTo?: string
  initialBody?: string
  language: string
  onClose: () => void
  onSaved: () => void
}

export function Composer(props: Props) {
  const wails = useWails()

  const [subject, setSubject] = createSignal(props.initialSubject || '')
  const [to, setTo] = createSignal(props.initialTo || '')
  const [body, setBody] = createSignal(props.initialBody || '<p></p>')

  let editorRef: HTMLDivElement | undefined
  let saveTimer: number

  const draftId = props.draftId || 'draft-' + Date.now()
  const accountId = () => props.accountId || 'demo-local'

  // Real autosave to SQLite
  const scheduleAutosave = () => {
    clearTimeout(saveTimer)
    saveTimer = window.setTimeout(async () => {
      try {
        await wails.SaveDraft({
          id: draftId,
          accountId: accountId(),
          subject: subject(),
          to: JSON.stringify([to()]),
          bodyHtml: body(),
        })
        props.onSaved()
      } catch (e) {
        console.error('Real draft autosave failed', e)
      }
    }, 800)
  }

  const handleInput = (e: InputEvent) => {
    const html = (e.currentTarget as HTMLDivElement).innerHTML
    setBody(html)
    scheduleAutosave()
  }

  const handleSubjectChange = (e: InputEvent) => {
    setSubject((e.currentTarget as HTMLInputElement).value)
    scheduleAutosave()
  }

  const handleToChange = (e: InputEvent) => {
    setTo((e.currentTarget as HTMLInputElement).value)
    scheduleAutosave()
  }

  // Basic rich formatting (real DOM commands)
  const exec = (cmd: string, val?: string) => {
    document.execCommand(cmd, false, val)
    if (editorRef) {
      setBody(editorRef.innerHTML)
      scheduleAutosave()
    }
  }

  const handleSend = async () => {
    try {
      await wails.SaveDraft({
        id: draftId,
        accountId: accountId(),
        subject: subject(),
        to: JSON.stringify([to()]),
        bodyHtml: body(),
      })
      // Real local send: creates actual message row in Sent folder and removes draft
      await wails.SendDraft?.(draftId)
      props.onClose()
    } catch (e) {
      console.error('Real send failed', e)
    }
  }

  const handleDiscard = async () => {
    try {
      await wails.DeleteDraft?.(draftId) // Will be wired
    } catch {}
    props.onClose()
  }

  onCleanup(() => clearTimeout(saveTimer))

  return (
    <Dialog.Root open={true} onOpenChange={(e) => !e.open && props.onClose()}>
      <Dialog.Backdrop class="fixed inset-0 bg-black/40 z-[100]" />
      <Dialog.Positioner class="fixed inset-0 flex items-center justify-center z-[101]">
        <Dialog.Content class="composer-dialog">
        <div class="composer-header">
          <div class="font-semibold">{t(props.language, 'newDraft')}</div>
          <div class="composer-actions">
            <button onClick={handleDiscard} class="danger-button">{t(props.language, 'discard')}</button>
            <button onClick={handleSend} class="primary-button">{t(props.language, 'sendLocal')}</button>
            <button onClick={props.onClose} class="icon-button">×</button>
          </div>
        </div>

        <div class="composer-body">
          <input
            class="form-input"
            placeholder={t(props.language, 'to')}
            value={to()}
            onInput={handleToChange}
          />
          <input
            class="form-input subject-input"
            placeholder={t(props.language, 'subject')}
            value={subject()}
            onInput={handleSubjectChange}
          />

          <div class="composer-toolbar">
            <button onClick={() => exec('bold')} class="tool-button font-bold" title="Bold">B</button>
            <button onClick={() => exec('italic')} class="tool-button italic" title="Italic">I</button>
            <button onClick={() => exec('insertUnorderedList')} class="tool-button" title="Bulleted list">•</button>
            <button onClick={() => exec('insertOrderedList')} class="tool-button" title="Numbered list">1</button>
          </div>

          <div
            ref={editorRef}
            contentEditable
            class="composer-editor"
            innerHTML={body()}
            onInput={handleInput}
          />
        </div>

        <div class="composer-footer">Autosaved locally</div>
        </Dialog.Content>
      </Dialog.Positioner>
    </Dialog.Root>
  )
}
