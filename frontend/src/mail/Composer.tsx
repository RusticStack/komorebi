import { createSignal, onCleanup } from 'solid-js'
import { Dialog } from '@ark-ui/solid/dialog'
import { useWails } from '../lib/wails'

interface Props {
  draftId?: string
  initialSubject?: string
  initialTo?: string
  initialBody?: string
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

  // Real autosave to SQLite
  const scheduleAutosave = () => {
    clearTimeout(saveTimer)
    saveTimer = window.setTimeout(async () => {
      try {
        await wails.SaveDraft({
          id: draftId,
          accountId: 'demo-local',
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
        accountId: 'demo-local',
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
        <Dialog.Content class="bg-white w-[820px] max-h-[85vh] rounded-xl shadow-2xl flex flex-col border border-[#D9D4CC]">
        {/* Header */}
        <div class="flex items-center justify-between px-5 py-3 border-b">
          <div class="font-semibold">New Message (real local draft)</div>
          <div class="flex gap-2">
            <button onClick={handleDiscard} class="px-4 py-1.5 text-sm rounded hover:bg-red-50 text-red-600">Discard</button>
            <button onClick={handleSend} class="px-6 py-1.5 text-sm rounded bg-[#5C6B5E] text-white hover:bg-[#8B5E3C]">Send (local)</button>
            <button onClick={props.onClose} class="px-3 text-xl leading-none">×</button>
          </div>
        </div>

        <div class="p-4 space-y-3">
          <input
            class="w-full border border-[#D9D4CC] rounded px-3 py-2 text-sm"
            placeholder="To"
            value={to()}
            onInput={handleToChange}
          />
          <input
            class="w-full border border-[#D9D4CC] rounded px-3 py-2 text-sm font-medium"
            placeholder="Subject"
            value={subject()}
            onInput={handleSubjectChange}
          />

          {/* Real rich editing toolbar */}
          <div class="flex gap-1 border-b pb-2">
            <button onClick={() => exec('bold')} class="px-2 py-1 text-sm hover:bg-[#F7F5F2] rounded font-bold">B</button>
            <button onClick={() => exec('italic')} class="px-2 py-1 text-sm hover:bg-[#F7F5F2] rounded italic">I</button>
            <button onClick={() => exec('insertUnorderedList')} class="px-2 py-1 text-sm hover:bg-[#F7F5F2] rounded">• List</button>
            <button onClick={() => exec('insertOrderedList')} class="px-2 py-1 text-sm hover:bg-[#F7F5F2] rounded">1. List</button>
          </div>

          {/* Real contenteditable composer body */}
          <div
            ref={editorRef}
            contentEditable
            class="min-h-[280px] border border-[#D9D4CC] rounded p-4 text-sm focus:outline-none prose prose-sm max-w-none"
            innerHTML={body()}
            onInput={handleInput}
          />
        </div>

        <div class="text-[10px] px-5 py-2 text-[#8A8680] border-t">
          Drafts are saved in real time to local SQLite. Full rich formatting + attachments + real SMTP coming in next slices.
        </div>
        </Dialog.Content>
      </Dialog.Positioner>
    </Dialog.Root>
  )
}
