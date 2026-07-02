import React from 'react'
import ReactDOM from 'react-dom/client'
import { App } from './App'
import { PanelWindow } from './PanelWindow'
import './mockup.css'

// A detached panel window is the same bundle opened with `?window=<panel>`
// (see the open_panel_window command). Render just that panel; otherwise the
// full app.
const params = new URLSearchParams(window.location.search)
const panelWindow = params.get('window')

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    {panelWindow ? <PanelWindow panel={panelWindow} params={params} /> : <App />}
  </React.StrictMode>,
)
