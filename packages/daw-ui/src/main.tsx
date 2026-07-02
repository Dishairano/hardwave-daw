import React from 'react'
import ReactDOM from 'react-dom/client'
import { App } from './App'
import { PanelWindow } from './PanelWindow'
import './mockup.css'

// A detached panel window is the same bundle opened with
// `index.html#window=<panel>&trackId=..&clipId=..` (see the open_panel_window
// command). We read the panel from the URL HASH (a query string on the window
// URL can break asset loading → blank window). Fall back to ?query for safety.
const raw = window.location.hash.startsWith('#')
  ? window.location.hash.slice(1)
  : window.location.search.replace(/^\?/, '')
const params = new URLSearchParams(raw)
const panelWindow = params.get('window')

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
