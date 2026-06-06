import { createSignal } from 'solid-js'
import { t } from '../i18n'

interface Props {
  onSearch: (query: string) => void
  language: string
}

export function SearchBar(props: Props) {
  const [value, setValue] = createSignal('')

  let timeout: number

  const handleInput = (e: InputEvent) => {
    const v = (e.currentTarget as HTMLInputElement).value
    setValue(v)

    clearTimeout(timeout)
    timeout = window.setTimeout(() => {
      props.onSearch(v)
    }, 280)
  }

  return (
    <div class="search-shell">
      <input
        type="text"
        class="search-input"
        placeholder={t(props.language, 'search')}
        value={value()}
        onInput={handleInput}
      />
    </div>
  )
}
