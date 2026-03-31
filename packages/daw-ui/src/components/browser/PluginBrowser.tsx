import { usePluginStore } from '../../stores/pluginStore'
import { useTrackStore } from '../../stores/trackStore'

export function PluginBrowser() {
  const { plugins, scanning, scanPlugins } = usePluginStore()
  const { selectedTrackId } = useTrackStore()

  return (
    <div style={{
      background: '#0e0e10',
      borderLeft: '1px solid rgba(255,255,255,0.06)',
      display: 'flex',
      flexDirection: 'column',
      overflow: 'hidden',
    }}>
      <div style={{
        padding: '8px 12px',
        borderBottom: '1px solid rgba(255,255,255,0.04)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
      }}>
        <span style={{ fontSize: 10, fontWeight: 700, color: '#555', letterSpacing: 1 }}>PLUGINS</span>
        <button
          onClick={scanPlugins}
          disabled={scanning}
          style={{
            padding: '2px 8px', fontSize: 9,
            background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.08)',
            borderRadius: 4, color: '#666', cursor: 'pointer', fontFamily: 'inherit',
          }}
        >
          {scanning ? 'Scanning...' : 'Scan'}
        </button>
      </div>

      <div style={{ flex: 1, overflowY: 'auto', padding: '4px 0' }}>
        {plugins.length === 0 && (
          <div style={{ padding: 16, textAlign: 'center', color: '#222', fontSize: 10 }}>
            Click "Scan" to find<br />VST3 & CLAP plugins
          </div>
        )}

        {plugins.map((plugin) => (
          <div
            key={plugin.id}
            style={{
              padding: '5px 12px',
              cursor: selectedTrackId ? 'pointer' : 'default',
              opacity: selectedTrackId ? 1 : 0.5,
            }}
            onClick={async () => {
              if (!selectedTrackId) return
              const { addToTrack } = usePluginStore.getState()
              await addToTrack(selectedTrackId, plugin.id)
              useTrackStore.getState().fetchTracks()
            }}
            onMouseEnter={(e) => {
              if (selectedTrackId) (e.currentTarget as HTMLElement).style.background = 'rgba(255,255,255,0.03)'
            }}
            onMouseLeave={(e) => {
              (e.currentTarget as HTMLElement).style.background = 'transparent'
            }}
          >
            <div style={{ fontSize: 11, color: '#ccc' }}>{plugin.name}</div>
            <div style={{ fontSize: 9, color: '#444' }}>
              {plugin.vendor} — {plugin.format}
            </div>
          </div>
        ))}
      </div>

      {selectedTrackId && plugins.length > 0 && (
        <div style={{ padding: '6px 12px', borderTop: '1px solid rgba(255,255,255,0.04)', fontSize: 9, color: '#333' }}>
          Click a plugin to add to selected track
        </div>
      )}
    </div>
  )
}
