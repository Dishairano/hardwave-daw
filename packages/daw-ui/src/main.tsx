import React from 'react'
import ReactDOM from 'react-dom/client'
import { App } from './App'
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

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    {panelWindow ? (
      <PanelErrorBoundary>
        <PanelWindow panel={panelWindow} params={params} />
      </PanelErrorBoundary>
    ) : (
      <App />
    )}
  </React.StrictMode>,
)
