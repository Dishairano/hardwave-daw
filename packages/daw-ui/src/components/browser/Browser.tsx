import { useState } from 'react'
import { hw } from '../../theme'
import { usePluginStore } from '../../stores/pluginStore'
import { useTrackStore } from '../../stores/trackStore'

type Tab = 'plugins' | 'files' | 'project'

export function Browser() {
  const [activeTab, setActiveTab] = useState<Tab>('plugins')
  const { plugins, scanning, scanPlugins } = usePluginStore()
  const { selectedTrackId } = useTrackStore()

  return (
    <div style={{
      width: 200,
      minWidth: 200,
      background: hw.bgElevated,
      borderRight: `1px solid ${hw.border}`,
      display: 'flex',
      flexDirection: 'column',
    }}>
      {/* Tab bar */}
      <div style={{
        display: 'flex',
        background: hw.bg,
        borderBottom: `1px solid ${hw.border}`,
        height: 30,
      }}>
        {(['plugins', 'files', 'project'] as Tab[]).map(tab => (
          <button
            key={tab}
            onClick={() => setActiveTab(tab)}
            style={{
              flex: 1,
              fontSize: 11,
              fontWeight: 500,
              color: activeTab === tab ? hw.textPrimary : hw.textFaint,
              position: 'relative',
            }}
          >
            {tab.charAt(0).toUpperCase() + tab.slice(1)}
            {activeTab === tab && (
              <div style={{
                position: 'absolute', bottom: 0, left: 6, right: 6,
                height: 2, background: hw.red, borderRadius: 1,
              }} />
            )}
          </button>
        ))}
      </div>

      {/* Content */}
      <div style={{ flex: 1, overflowY: 'auto', padding: 4 }}>
        {activeTab === 'plugins' && (
          <>
            <button
              onClick={scanPlugins}
              disabled={scanning}
              style={{
                width: '100%', padding: '6px 0', margin: '4px 0 8px',
                fontSize: 11, color: hw.textSecondary,
                background: hw.bgCard,
                border: `1px solid ${hw.border}`,
                borderRadius: hw.radius.md,
                opacity: scanning ? 0.5 : 1,
              }}
            >
              {scanning ? 'Scanning...' : 'Scan Plugins'}
            </button>

            {plugins.length === 0 && (
              <div style={{ padding: 16, textAlign: 'center', color: hw.textFaint, fontSize: 11 }}>
                Click Scan to find<br />VST3 & CLAP plugins
              </div>
            )}

            {plugins.map((plugin) => (
              <div
                key={plugin.id}
                style={{
                  padding: '6px 8px',
                  borderRadius: hw.radius.sm,
                  marginBottom: 1,
                  cursor: selectedTrackId ? 'pointer' : 'default',
                  opacity: selectedTrackId ? 1 : 0.5,
                }}
                onClick={async () => {
                  if (!selectedTrackId) return
                  const { addToTrack } = usePluginStore.getState()
                  await addToTrack(selectedTrackId, plugin.id)
                  useTrackStore.getState().fetchTracks()
                }}
                onMouseEnter={e => { e.currentTarget.style.background = hw.bgHover }}
                onMouseLeave={e => { e.currentTarget.style.background = 'transparent' }}
              >
                <div style={{ fontSize: 11, color: hw.textSecondary }}>{plugin.name}</div>
                <div style={{ fontSize: 9, color: hw.textFaint, marginTop: 1 }}>
                  {plugin.vendor} · {plugin.format}
                </div>
              </div>
            ))}
          </>
        )}

        {activeTab === 'files' && (
          <div style={{ padding: 16, textAlign: 'center', color: hw.textFaint, fontSize: 11 }}>
            Drag audio files onto<br />the playlist to import
          </div>
        )}

        {activeTab === 'project' && (
          <div style={{ padding: 16, textAlign: 'center', color: hw.textFaint, fontSize: 11 }}>
            Project files will<br />appear here
          </div>
        )}
      </div>
    </div>
  )
}
