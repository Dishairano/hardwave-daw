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
      width: 200,
      minWidth: 200,
      background: '#232323',
      borderRight: '2px solid #1A1A1A',
      display: 'flex',
      flexDirection: 'column',
    }}>
      {/* Tab bar */}
      <div style={{
        display: 'flex',
        background: '#1E1E1E',
        borderBottom: '1px solid #333',
      }}>
        {(['plugins', 'files', 'current'] as Tab[]).map(tab => (
          <div
            key={tab}
            onClick={() => setActiveTab(tab)}
            style={{
              flex: 1,
              padding: '6px 0',
              textAlign: 'center',
              fontSize: 9,
              fontWeight: 700,
              color: activeTab === tab ? '#FF6B00' : '#666',
              background: activeTab === tab ? '#2A2A2A' : 'transparent',
              borderBottom: activeTab === tab ? '2px solid #FF6B00' : '2px solid transparent',
              cursor: 'pointer',
              letterSpacing: 0.5,
              textTransform: 'uppercase',
            }}
          >
            {tab === 'current' ? 'Proj' : tab}
          </div>
        ))}
      </div>

      {/* Content */}
      <div style={{ flex: 1, overflowY: 'auto' }}>
        {activeTab === 'plugins' && (
          <div>
            {/* Scan button */}
            <div style={{
              padding: '6px 8px',
              borderBottom: '1px solid #333',
            }}>
              <button
                onClick={scanPlugins}
                disabled={scanning}
                style={{
                  width: '100%',
                  padding: '4px 0',
                  fontSize: 10,
                  fontWeight: 600,
                  color: '#AAA',
                  background: '#2E2E2E',
                  border: '1px solid #444',
                  borderRadius: 3,
                }}
              >
                {scanning ? 'Scanning...' : 'Scan for Plugins'}
              </button>
            </div>

            {/* Plugin list */}
            {plugins.length === 0 && (
              <div style={{ padding: 16, textAlign: 'center', color: '#444', fontSize: 10 }}>
                Click Scan to find<br />VST3 & CLAP plugins
              </div>
            )}

            {plugins.map((plugin) => (
              <div
                key={plugin.id}
                style={{
                  padding: '4px 8px',
                  cursor: selectedTrackId ? 'pointer' : 'default',
                  opacity: selectedTrackId ? 1 : 0.6,
                  borderBottom: '1px solid #2A2A2A',
                }}
                onClick={async () => {
                  if (!selectedTrackId) return
                  const { addToTrack } = usePluginStore.getState()
                  await addToTrack(selectedTrackId, plugin.id)
                  useTrackStore.getState().fetchTracks()
                }}
                onMouseEnter={e => { if (selectedTrackId) e.currentTarget.style.background = '#333' }}
                onMouseLeave={e => { e.currentTarget.style.background = 'transparent' }}
              >
                <div style={{ fontSize: 10, color: '#CCC' }}>{plugin.name}</div>
                <div style={{ fontSize: 8, color: '#555' }}>
                  {plugin.vendor} | {plugin.format}
                </div>
              </div>
            ))}
          </div>
        )}

        {activeTab === 'files' && (
          <div style={{ padding: 16, textAlign: 'center', color: '#444', fontSize: 10 }}>
            Drag audio files onto<br />the Playlist to import
          </div>
        )}

        {activeTab === 'current' && (
          <div style={{ padding: 16, textAlign: 'center', color: '#444', fontSize: 10 }}>
            Project files will<br />appear here
          </div>
        )}
      </div>
    </div>
  )
}
