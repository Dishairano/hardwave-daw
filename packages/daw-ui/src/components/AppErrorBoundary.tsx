import React from 'react'

/**
 * Last-resort boundary around the whole app. A render crash previously
 * left a frozen/white window with nothing actionable (deep-research
 * P1-8); now the user gets an explanation, the error text to paste into
 * a bug report, and a reload button — the Rust side (and their project)
 * is untouched by a webview render crash, so reloading is safe.
 */
export class AppErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { err: Error | null }
> {
  state: { err: Error | null } = { err: null }

  static getDerivedStateFromError(err: Error) {
    return { err }
  }

  componentDidCatch(err: Error, info: React.ErrorInfo) {
    console.error('App render crash:', err, info.componentStack)
  }

  render() {
    if (!this.state.err) return this.props.children
    const text = this.state.err.stack || this.state.err.message || String(this.state.err)
    return (
      <div style={{
        height: '100vh', background: '#08080c', color: '#d4d4d8',
        display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center',
        gap: 14, padding: 32, fontFamily: 'Inter, ui-sans-serif, system-ui, sans-serif',
      }}>
        <div style={{ fontSize: 18, fontWeight: 800, color: '#fafafa' }}>
          The interface hit an error
        </div>
        <div style={{ fontSize: 13, color: '#a1a1a6', maxWidth: 520, textAlign: 'center' }}>
          Your project and the audio engine are unaffected. Reload the interface to continue —
          if this keeps happening, copy the details below into a Discord bug report
          (Help → Export diagnostics attaches the session log).
        </div>
        <button
          onClick={() => window.location.reload()}
          style={{
            padding: '8px 22px', fontSize: 13, fontWeight: 700, cursor: 'pointer',
            background: '#DC2626', color: '#fff', border: 'none', borderRadius: 8,
          }}
        >
          Reload interface
        </button>
        <pre style={{
          maxWidth: 720, maxHeight: '38vh', overflow: 'auto', margin: 0,
          padding: 14, fontSize: 11, whiteSpace: 'pre-wrap',
          background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.09)',
          borderRadius: 8, color: '#ff8a8a',
        }}>
          {text}
        </pre>
      </div>
    )
  }
}
