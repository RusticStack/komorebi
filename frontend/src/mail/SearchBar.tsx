import { createSignal } from 'solid-js'

interface Props {
  onSearch: (query: string) => void
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
    <div class="p-3 border-b border-[#D9D4CC] bg-[#F1EDE7]">
      <input
        type="text"
        class="search-input"
        placeholder="Search real local cache (FTS5)..."
        value={value()}
        onInput={handleInput}
      />
    </div>
  )
}
