import { createMemo, createSignal, For, onCleanup, Show } from 'solid-js'
import { t } from '../i18n'
import { useWails } from '../lib/wails'
import { DiscoveredAccountSettings, OAuthStartResponse, ProviderPolicy, ValidationCheck } from '../types/mail'

interface Props {
  open: boolean
  policies: ProviderPolicy[]
  language: string
  onClose: () => void
  onChanged: () => void
}

export function AccountSetupPanel(props: Props) {
  const wails = useWails()
  const [email, setEmail] = createSignal('')
  const [displayName, setDisplayName] = createSignal('')
  const [password, setPassword] = createSignal('')
  const [providerKind, setProviderKind] = createSignal('generic')
  const [providerLabel, setProviderLabel] = createSignal('Generic IMAP/SMTP')
  const [authMode, setAuthMode] = createSignal('password')
  const [imapHost, setImapHost] = createSignal('')
  const [imapPort, setImapPort] = createSignal(993)
  const [imapSecurity, setImapSecurity] = createSignal('tls')
  const [smtpHost, setSmtpHost] = createSignal('')
  const [smtpPort, setSmtpPort] = createSignal(587)
  const [smtpSecurity, setSmtpSecurity] = createSignal('starttls')
  const [clientId, setClientId] = createSignal('')
  const [redirectUri, setRedirectUri] = createSignal('http://127.0.0.1:4545/oauth/callback')
  const [authCode, setAuthCode] = createSignal('')
  const [oauth, setOAuth] = createSignal<OAuthStartResponse | null>(null)
  const [candidates, setCandidates] = createSignal<DiscoveredAccountSettings[]>([])
  const [diagnostics, setDiagnostics] = createSignal<{ source: string; status: string; detail: string }[]>([])
  const [validationChecks, setValidationChecks] = createSignal<ValidationCheck[]>([])
  const [selectedKey, setSelectedKey] = createSignal('')
  const [busy, setBusy] = createSignal(false)
  const [status, setStatus] = createSignal('')

  const isOAuth = createMemo(() => authMode() === 'oauth')
  const oauthSupported = createMemo(() => ['gmail', 'microsoft'].includes(providerKind()))
  const selectedPolicy = createMemo(() => props.policies.find((item) => item.providerKind === providerKind()))
  let oauthPollTimer: number | undefined

  onCleanup(() => {
    if (oauthPollTimer !== undefined) {
      window.clearTimeout(oauthPollTimer)
    }
  })

  const candidateKey = (candidate: DiscoveredAccountSettings) => [
    candidate.source,
    candidate.providerKind,
    candidate.imapHost,
    candidate.imapPort,
    candidate.smtpHost,
    candidate.smtpPort,
  ].join('|')

  const applyCandidate = (candidate: DiscoveredAccountSettings) => {
    setProviderKind(candidate.providerKind)
    setProviderLabel(candidate.providerLabel)
    setAuthMode(candidate.authMode)
    setImapHost(candidate.imapHost)
    setImapPort(candidate.imapPort)
    setImapSecurity(candidate.imapSecurity)
    setSmtpHost(candidate.smtpHost)
    setSmtpPort(candidate.smtpPort)
    setSmtpSecurity(candidate.smtpSecurity)
    setSelectedKey(candidateKey(candidate))
    setOAuth(null)
    setAuthCode('')
    setStatus('')
    setValidationChecks([])
  }

  const discover = async () => {
    setBusy(true)
    setStatus('')
    setOAuth(null)
    setValidationChecks([])
    try {
      const result = await wails.DiscoverAccountSettings({ email: email() })
      setCandidates(result.candidates)
      setDiagnostics(result.diagnostics)
      if (result.candidates.length > 0) {
        applyCandidate(result.candidates[0])
        setStatus(`${result.candidates.length} ${t(props.language, 'settingsFound')}`)
      } else {
        setStatus(t(props.language, 'noSettingsFound'))
      }
    } catch (error) {
      setStatus(error instanceof Error ? error.message : String(error))
    } finally {
      setBusy(false)
    }
  }

  const useManual = () => {
    const domain = email().includes('@') ? email().split('@').pop() ?? '' : ''
    setProviderKind('generic')
    setProviderLabel('Manual IMAP/SMTP')
    setAuthMode('password')
    setImapHost(domain ? `mail.${domain}` : '')
    setImapPort(993)
    setImapSecurity('tls')
    setSmtpHost(domain ? `mail.${domain}` : '')
    setSmtpPort(465)
    setSmtpSecurity('tls')
    setSelectedKey('manual')
    setOAuth(null)
    setStatus('')
    setValidationChecks([])
  }

  const startOAuth = async () => {
    setStatus('')
    const policy = selectedPolicy()
    if (!policy || !oauthSupported()) {
      setStatus(t(props.language, 'oauthUnsupported'))
      return
    }
    const result = await wails.StartOAuthAccountSetup({
      providerKind: providerKind(),
      email: email(),
      displayName: displayName(),
      clientId: clientId(),
      redirectUri: redirectUri(),
    })
    setOAuth(result)
    if (result.authUrl) {
      window.open(result.authUrl, '_blank', 'noopener,noreferrer')
    }
    if (result.callbackMode === 'loopback') {
      setStatus(t(props.language, 'waitingOAuth'))
      pollOAuthCallback(result)
    }
  }

  const pollOAuthCallback = (current: OAuthStartResponse) => {
    if (oauthPollTimer !== undefined) {
      window.clearTimeout(oauthPollTimer)
    }
    oauthPollTimer = window.setTimeout(async () => {
      try {
        const result = await wails.GetOAuthCallbackStatus({ state: current.state })
        if (result.complete) {
          setStatus(result.accountId)
          props.onChanged()
          return
        }
        if (result.error) {
          setStatus(result.error)
          return
        }
        if (current.expiresAt && Date.now() / 1000 > current.expiresAt) {
          setStatus(t(props.language, 'oauthExpired'))
          return
        }
        pollOAuthCallback(current)
      } catch (error) {
        setStatus(error instanceof Error ? error.message : String(error))
      }
    }, 1200)
  }

  const completeOAuth = async () => {
    const current = oauth()
    if (!current) return
    const id = await wails.CompleteOAuthAccountSetup({
      state: current.state,
      code: authCode(),
      clientId: clientId(),
      redirectUri: redirectUri(),
    })
    setStatus(id)
    props.onChanged()
  }

  const passwordAccountRequest = () => ({
      email: email(),
      displayName: displayName(),
      providerKind: 'generic',
      authMode: 'password',
      password: password(),
      imapHost: imapHost(),
      imapPort: imapPort(),
      imapSecurity: imapSecurity(),
      smtpHost: smtpHost(),
      smtpPort: smtpPort(),
      smtpSecurity: smtpSecurity(),
  })

  const runPasswordValidation = async () => {
    setStatus(t(props.language, 'validatingAccount'))
    setValidationChecks([])
    const response = await wails.ValidateAccountSetup(passwordAccountRequest())
    setValidationChecks(response.checks ?? [])
    setStatus(response.ok ? t(props.language, 'validationPassed') : t(props.language, 'validationFailed'))
    return response.ok
  }

  const testPasswordAccount = async () => {
    setBusy(true)
    try {
      await runPasswordValidation()
    } catch (error) {
      setStatus(error instanceof Error ? error.message : String(error))
    } finally {
      setBusy(false)
    }
  }

  const savePasswordAccount = async () => {
    setBusy(true)
    try {
      const valid = await runPasswordValidation()
      if (!valid) return
      setStatus(t(props.language, 'saving'))
      const id = await wails.AddAccount(passwordAccountRequest())
      setStatus(id)
      setPassword('')
      props.onChanged()
    } catch (error) {
      setStatus(error instanceof Error ? error.message : String(error))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Show when={props.open}>
      <div class="panel-scrim" onClick={props.onClose} />
      <section class="setup-panel">
        <header class="panel-header">
          <h2>{t(props.language, 'addAccount')}</h2>
          <button class="icon-button" onClick={props.onClose}>×</button>
        </header>

        <div class="setup-body">
          <div class="form-grid">
            <label>
              <span>{t(props.language, 'email')}</span>
              <input value={email()} onInput={(event) => setEmail(event.currentTarget.value)} />
            </label>
            <label>
              <span>{t(props.language, 'displayName')}</span>
              <input value={displayName()} onInput={(event) => setDisplayName(event.currentTarget.value)} />
            </label>
          </div>

          <div class="setup-actions">
            <button class="primary-button" onClick={discover} disabled={busy()}>
              {busy() ? t(props.language, 'loadingShort') : t(props.language, 'discover')}
            </button>
            <button class="secondary-button" onClick={useManual}>{t(props.language, 'manualSettings')}</button>
          </div>

          <Show when={candidates().length > 0}>
            <div class="candidate-list">
              <For each={candidates()}>
                {(candidate) => (
                  <button
                    class={selectedKey() === candidateKey(candidate) ? 'candidate-card active' : 'candidate-card'}
                    onClick={() => applyCandidate(candidate)}
                  >
                    <div class="candidate-topline">
                      <strong>{candidate.providerLabel}</strong>
                      <span>{candidate.confidence}%</span>
                    </div>
                    <div class="candidate-source">{candidate.source}</div>
                    <div class="candidate-endpoints">
                      <span>IMAP {candidate.imapHost}:{candidate.imapPort}</span>
                      <span>SMTP {candidate.smtpHost}:{candidate.smtpPort}</span>
                    </div>
                    <Show when={candidate.warnings.length > 0}>
                      <div class="candidate-warning">{candidate.warnings[0]}</div>
                    </Show>
                  </button>
                )}
              </For>
            </div>
          </Show>

          <div class="settings-summary">
            <div>
              <span>{t(props.language, 'provider')}</span>
              <strong>{providerLabel()}</strong>
            </div>
            <div>
              <span>{t(props.language, 'auth')}</span>
              <strong>{authMode()}</strong>
            </div>
          </div>

          <div class="form-stack">
            <label>
              <span>{t(props.language, 'imapHost')}</span>
              <input value={imapHost()} onInput={(event) => setImapHost(event.currentTarget.value)} />
            </label>
            <div class="form-grid compact">
              <label>
                <span>{t(props.language, 'port')}</span>
                <input type="number" min="1" max="65535" value={imapPort()} onInput={(event) => setImapPort(Number(event.currentTarget.value))} />
              </label>
              <label>
                <span>{t(props.language, 'security')}</span>
                <select class="form-input" value={imapSecurity()} onChange={(event) => setImapSecurity(event.currentTarget.value)}>
                  <option value="tls">TLS</option>
                  <option value="starttls">STARTTLS</option>
                </select>
              </label>
            </div>
            <label>
              <span>{t(props.language, 'smtpHost')}</span>
              <input value={smtpHost()} onInput={(event) => setSmtpHost(event.currentTarget.value)} />
            </label>
            <div class="form-grid compact">
              <label>
                <span>{t(props.language, 'port')}</span>
                <input type="number" min="1" max="65535" value={smtpPort()} onInput={(event) => setSmtpPort(Number(event.currentTarget.value))} />
              </label>
              <label>
                <span>{t(props.language, 'security')}</span>
                <select class="form-input" value={smtpSecurity()} onChange={(event) => setSmtpSecurity(event.currentTarget.value)}>
                  <option value="tls">TLS</option>
                  <option value="starttls">STARTTLS</option>
                </select>
              </label>
            </div>
          </div>

          <Show when={isOAuth()} fallback={
            <div class="form-stack">
              <label>
                <span>{t(props.language, 'password')}</span>
                <input type="password" value={password()} onInput={(event) => setPassword(event.currentTarget.value)} />
              </label>
              <div class="setup-actions">
                <button class="secondary-button" onClick={testPasswordAccount} disabled={busy()}>{t(props.language, 'validate')}</button>
                <button class="primary-button" onClick={savePasswordAccount} disabled={busy()}>{t(props.language, 'save')}</button>
              </div>
            </div>
          }>
            <div class="form-stack">
              <label>
                <span>Client ID</span>
                <input value={clientId()} onInput={(event) => setClientId(event.currentTarget.value)} />
              </label>
              <label>
                <span>Redirect URI</span>
                <input value={redirectUri()} onInput={(event) => setRedirectUri(event.currentTarget.value)} />
              </label>
              <button class="primary-button" onClick={startOAuth}>{t(props.language, 'startOAuth')}</button>
              <Show when={oauth() && oauth()?.callbackMode !== 'loopback'}>
                <label>
                  <span>{t(props.language, 'authCode')}</span>
                  <input value={authCode()} onInput={(event) => setAuthCode(event.currentTarget.value)} />
                </label>
                <button class="secondary-button" onClick={completeOAuth}>{t(props.language, 'completeOAuth')}</button>
              </Show>
            </div>
          </Show>

          <Show when={validationChecks().length > 0}>
            <div class="validation-list">
              <For each={validationChecks()}>
                {(item) => (
                  <div class={`validation-row ${item.status}`}>
                    <strong>{item.label}</strong>
                    <span>{item.detail}</span>
                  </div>
                )}
              </For>
            </div>
          </Show>

          <Show when={diagnostics().length > 0}>
            <div class="diagnostic-chips">
              <For each={diagnostics().slice(0, 4)}>
                {(item) => <span class={item.status}>{item.source}: {item.status}</span>}
              </For>
            </div>
          </Show>

          <Show when={status()}>
            <div class="panel-status">{status()}</div>
          </Show>
        </div>
      </section>
    </Show>
  )
}
