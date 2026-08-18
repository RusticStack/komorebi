import { createSignal, For, Show } from 'solid-js'
import { DiagnosticsHealth } from '../types/mail'
import { t } from '../i18n'
import { useWails } from '../lib/wails'

interface Props {
  open: boolean
  health?: DiagnosticsHealth
  language: string
  onClose: () => void
}

export function DiagnosticsPanel(props: Props) {
  const wails = useWails()
  const [status, setStatus] = createSignal('')

  const exportDiagnostics = async () => {
    setStatus('')
    try {
      const result = await wails.ExportDiagnostics({ includeSchema: true })
      setStatus(result.path ? `${t(props.language, 'diagnosticsExported')}: ${result.path}` : t(props.language, 'diagnosticsExported'))
    } catch (error) {
      setStatus(error instanceof Error ? error.message : String(error))
    }
  }

  return (
    <Show when={props.open}>
      <div class="panel-scrim" onClick={props.onClose} />
      <section class="diagnostics-panel">
        <header class="panel-header">
          <h2>{t(props.language, 'diagnostics')}</h2>
          <button class="icon-button" onClick={props.onClose}>×</button>
        </header>

        <div class="metric-grid">
          <Metric label="Accounts" value={props.health?.accountCount ?? 0} />
          <Metric label="Messages" value={props.health?.messageCount ?? 0} />
          <Metric label="Drafts" value={props.health?.draftCount ?? 0} />
          <Metric label="OAuth" value={props.health?.pendingOAuth ?? 0} />
        </div>

        <div class="diagnostics-actions">
          <button class="secondary-button" onClick={exportDiagnostics}>{t(props.language, 'exportDiagnostics')}</button>
        </div>

        <Show when={status()}>
          <div class="panel-status">{status()}</div>
        </Show>

        <div class="check-list">
          <For each={props.health?.checks ?? []}>
            {(check) => (
              <div class={`check-row ${check.status}`}>
                <div>
                  <strong>{check.label}</strong>
                  <p>{check.detail}</p>
                </div>
                <span>{check.status}</span>
              </div>
            )}
          </For>
        </div>
      </section>
    </Show>
  )
}

function Metric(props: { label: string; value: number }) {
  return (
    <div class="metric">
      <span>{props.label}</span>
      <strong>{props.value}</strong>
    </div>
  )
}
