import React from 'react'
import ReactDOM from 'react-dom/client'

// Browser/dev mode (Playwright specs, `npm run dev` outside Tauri):
// install the mocked backend BEFORE anything imports @tauri-apps/api,
// or the app hangs on the splash waiting for IPC that will never
// answer. Real Tauri sets __TAURI_INTERNALS__ first, so this is a
// no-op in production. Static import keeps ordering deterministic;
// the module itself guards against overwriting a real backend.
import './dev/tauri-mock'
import { App } from './App'
import { AppErrorBoundary } from './components/AppErrorBoundary'
import { PanelWindow } from './PanelWindow'
import './mockup.css'

// A detached panel window (open_panel_window command) loads a plain
// index.html and receives its panel + context via an injected global
// (window.__HW_PANEL__), set by an initialization script BEFORE the page
// loads. This avoids putting anything in the URL, which can break asset
// resolution → blank white window. Fall back to hash/query for older builds.
const injected = (window as unknown as { __HW_PANEL__?: { panel?: string; params?: string } }).__HW_PANEL__
const rawFallback = window.location.hash.startsWith('#')
  ? window.location.hash.slice(1)
  : window.location.search.replace(/^\?/, '')
const params = new URLSearchParams(injected?.params ?? rawFallback)
// Detect the panel: injected global (primary) → hash/query → the OS window
// label (`panel-<slug>`, always correct). The label fallback guarantees a
// detached window never falls through to <App> (whose splash screen would
// hang forever = white window) even if the injected global is missing.
let panelWindow = injected?.panel ?? params.get('window') ?? null
if (!panelWindow) {
  try {
    const meta = (window as unknown as {
      __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string }; currentWebview?: { windowLabel?: string } } }
    }).__TAURI_INTERNALS__?.metadata
    const label = meta?.currentWindow?.label ?? meta?.currentWebview?.windowLabel
    if (typeof label === 'string' && label.startsWith('panel-')) {
      panelWindow = label.slice('panel-'.length)
    }
  } catch { /* not in Tauri */ }
}

// Error boundary so a crash in a detached panel shows a readable message
// instead of a white screen (and tells us exactly what failed).
class PanelErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { err: Error | null }
> {
  state: { err: Error | null } = { err: null }
  static getDerivedStateFromError(err: Error) {
    return { err }
  }
  render() {
    if (this.state.err) {
      return (
        <pre style={{ color: '#ff8a8a', background: '#08080c', padding: 20, margin: 0, height: '100vh', fontSize: 12, whiteSpace: 'pre-wrap', overflow: 'auto' }}>
          {`Detached panel failed to render:\n\n${this.state.err.stack || this.state.err.message || String(this.state.err)}`}
        </pre>
      )
    }
    return this.props.children
  }
}

// Report the routing decision to the remote diagnostic log (set by the panel
// window's init script) — tells us whether a detached window correctly picked
// PanelWindow or fell through to <App> (whose splash would hang white).
try {
  ;(window as unknown as { __HW_LOG__?: (t: string, d: unknown) => void }).__HW_LOG__?.('route', {
    panel: panelWindow,
    injected: !!injected,
    hasGlobal: !!(window as unknown as { __HW_PANEL__?: unknown }).__HW_PANEL__,
  })
} catch { /* not a panel window */ }

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    {panelWindow ? (
      <PanelErrorBoundary>
        <PanelWindow panel={panelWindow} params={params} />
      </PanelErrorBoundary>
    ) : (
      <AppErrorBoundary>
        <App />
      </AppErrorBoundary>
    )}
  </React.StrictMode>,
)
