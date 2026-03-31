import { useState } from 'react'
import { usePluginStore } from '../../stores/pluginStore'
import { useTrackStore } from '../../stores/trackStore'

type Tab = 'plugins' | 'files' | 'current'

export function Browser() {
  const [activeTab, setActiveTab] = useState<Tab>('plugins')
  const { plugins, scanning, scanPlugins } = usePluginStore()
  const { selectedTrackId } = useTrackStore()

  return (
    <div style={{
      width: 190,
      minWidth: 190,
      background: '#1D1D1D',
      borderRight: '1px solid #111',
      display: 'flex',
      flexDirection: 'column',
    }}>
      {/* Tab bar — FL uses icon tabs at top */}
      <div style={{
        display: 'flex',
        background: '#252525',
        borderBottom: '1px solid #111',
        height: 22,
      }}>
        {(['plugins', 'files', 'current'] as Tab[]).map(tab => (
          <div
            key={tab}
            onClick={() => setActiveTab(tab)}
            style={{
              flex: 1,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              fontSize: 9,
              fontWeight: 600,
              color: activeTab === tab ? '#E8A030' : '#666',
              background: activeTab === tab ? '#2A2A2A' : 'transparent',
              borderBottom: activeTab === tab ? '1px solid #E8A030' : '1px solid transparent',
              cursor: 'default',
              textTransform: 'uppercase',
              letterSpacing: 0.3,
            }}
          >
            {tab === 'current' ? 'Proj' : tab === 'plugins' ? 'Plug' : 'File'}
          </div>
        ))}
      </div>

      {/* Content */}
      <div style={{ flex: 1, overflowY: 'auto' }}>
        {activeTab === 'plugins' && (
          <div>
            {/* Scan button */}
            <div style={{ padding: '4px 6px', borderBottom: '1px solid #222' }}>
              <button
                onClick={scanPlugins}
                disabled={scanning}
                style={{
                  width: '100%',
                  padding: '3px 0',
                  fontSize: 10,
                  color: '#999',
                  background: '#252525',
                  border: '1px solid #3A3A3A',
                }}
              >
                {scanning ? 'Scanning...' : 'Scan Plugins'}
              </button>
            </div>

            {plugins.length === 0 && (
              <div style={{ padding: 12, textAlign: 'center', color: '#444', fontSize: 10 }}>
                Click Scan to find<br />VST3 & CLAP plugins
              </div>
            )}

            {plugins.map((plugin) => (
              <div
                key={plugin.id}
                style={{
                  padding: '3px 6px',
                  cursor: selectedTrackId ? 'default' : 'default',
                  opacity: selectedTrackId ? 1 : 0.5,
                  borderBottom: '1px solid #1A1A1A',
                }}
                onClick={async () => {
                  if (!selectedTrackId) return
                  const { addToTrack } = usePluginStore.getState()
                  await addToTrack(selectedTrackId, plugin.id)
                  useTrackStore.getState().fetchTracks()
                }}
                onMouseEnter={e => { if (selectedTrackId) e.currentTarget.style.background = '#282828' }}
                onMouseLeave={e => { e.currentTarget.style.background = 'transparent' }}
              >
                <div style={{ fontSize: 10, color: '#BBB' }}>{plugin.name}</div>
                <div style={{ fontSize: 8, color: '#555' }}>
                  {plugin.vendor} | {plugin.format}
                </div>
              </div>
            ))}
          </div>
        )}

        {activeTab === 'files' && (
          <div style={{ padding: 12, textAlign: 'center', color: '#444', fontSize: 10 }}>
            Drag audio files onto<br />the Playlist to import
          </div>
        )}

        {activeTab === 'current' && (
          <div style={{ padding: 12, textAlign: 'center', color: '#444', fontSize: 10 }}>
            Project files will<br />appear here
          </div>
        )}
      </div>
    </div>
  )
}
