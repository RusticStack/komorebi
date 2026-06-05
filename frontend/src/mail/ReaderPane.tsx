import { MessageRow } from '../types/mail'
import { formatFullDate } from '../lib/format'

interface Props {
  message: MessageRow
}

export function ReaderPane(props: Props) {
  const msg = props.message

  return (
    <div class="reader max-w-[720px] mx-auto">
      <div class="mb-6">
        <h1 class="text-2xl font-semibold tracking-tight">{msg.subject}</h1>
        <div class="mt-3 text-sm text-[#5C5A55]">
          From: <span class="font-medium">{msg.senderName}</span> &lt;{msg.senderEmail}&gt;
        </div>
        <div class="text-xs text-[#8A8680] mt-0.5">
          {formatFullDate(msg.dateReceived)}
        </div>
      </div>

      <div class="prose prose-sm max-w-none text-[#2D2D2D] leading-relaxed">
        <p>{msg.snippet || 'This message body is stored in the local SQLite cache.'}</p>
        <p class="text-[#8A8680] text-xs mt-8">
          Real persisted message (id: {msg.id}). No remote content loaded.
        </p>
      </div>
    </div>
  )
}
