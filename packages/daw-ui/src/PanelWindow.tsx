/**
 * Detached-panel view. When the app is opened with `?window=<panel>` (a real
 * OS window spawned by the `open_panel_window` command), main.tsx renders this
 * instead of the full <App>, so the window shows just that panel — draggable
 * to another monitor.
 *
 * It reuses the same live-state wiring as the main window: transport + meter
 * listeners (events broadcast to all windows via app.emit) and fetchTracks().
 * The piano roll's open clip is passed via ?trackId=&clipId=. Structural edits
 * persist through the shared backend; instant cross-window refresh of *other*
 * windows is a follow-up.
 */
import { useEffect, useState } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { emit } from '@tauri-apps/api/event'
import { useTrackStore } from './stores/trackStore'
import { useTransportStore } from './stores/transportStore'
import { useMeterStore } from './stores/meterStore'
import { PianoRoll } from './components/piano-roll/PianoRoll'
import { MixerPanel } from './components/mixer/MixerPanel'
import { ChannelRack } from './components/channelrack/ChannelRack'
import { Arrangement } from './components/arrangement/Arrangement'
import { Browser } from './components/browser/Browser'
import { hw } from './theme'

const TITLES: Record<string, string> = {
  pianoRoll: 'Piano Roll', mixer: 'Mixer', channelRack: 'Channel Rack',
  playlist: 'Playlist', browser: 'Browser',
}

export function PanelWindow({ panel, params }: { panel: string; params: URLSearchParams }) {
  const [ready, setReady] = useState(false)

  useEffect(() => {
    // Live playback/meters: these subscribe to daw:transport / daw:meters,
    // which the engine emits to every window via app.emit — so the detached
    // window follows the playhead without extra plumbing.
    try { useTransportStore.getState().startListening() } catch { /* noop */ }
    try { useMeterStore.getState().startListening() } catch { /* noop */ }

    const trackId = params.get('trackId')
    const clipId = params.get('clipId')
    Promise.resolve(useTrackStore.getState().fetchTracks())
      .then(() => {
        if (trackId && clipId) {
          useTrackStore.setState({ activeMidiTrackId: trackId, activeMidiClipId: clipId } as never)
        }
      })
      .catch(() => { /* backend may be mid-init */ })
      .finally(() => setReady(true))
  }, [params])

  const dock = async () => {
    // Tell the main window to re-show its inline copy, then close this window.
    try { await emit('daw:dockPanel', { panel }) } catch { /* noop */ }
    try { await getCurrentWindow().close() } catch { /* noop */ }
  }

  const view = () => {
    switch (panel) {
      case 'pianoRoll': return <PianoRoll />
      case 'mixer': return <MixerPanel />
      case 'channelRack': return <ChannelRack />
      case 'playlist': return <Arrangement />
      case 'browser': return <Browser />
      default: return <div style={{ padding: 16, color: hw.textMuted }}>Unknown panel: {panel}</div>
    }
  }

  return (
    <div style={{ width: '100vw', height: '100vh', display: 'flex', flexDirection: 'column', background: '#08080c', overflow: 'hidden' }}>
      <div style={{
        height: 26, flexShrink: 0, display: 'flex', alignItems: 'center', gap: 8,
        padding: '0 8px', background: 'rgba(255,255,255,0.04)',
        borderBottom: `1px solid ${hw.border}`, fontSize: 10, fontWeight: 600, color: hw.textSecondary,
      }}>
        <span style={{ flex: 1 }}>{TITLES[panel] ?? panel} — detached</span>
        <button onClick={dock} title="Dock back into the main window"
          style={{ height: 18, padding: '0 8px', fontSize: 10, color: hw.textMuted, background: 'transparent', border: `1px solid ${hw.border}`, borderRadius: 4, cursor: 'pointer' }}>
          ⧉ Dock
        </button>
      </div>
      <div style={{ flex: 1, minHeight: 0, display: 'flex', overflow: 'hidden' }}>
        {ready ? view() : <div style={{ margin: 'auto', color: hw.textFaint, fontSize: 12 }}>Loading…</div>}
      </div>
    </div>
  )
}
