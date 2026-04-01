import { useState } from 'react'
import { hw } from '../../theme'
import { usePluginStore } from '../../stores/pluginStore'
import { useTrackStore } from '../../stores/trackStore'

type Tab = 'plugins' | 'files' | 'project'

export function Browser() {
  const [activeTab, setActiveTab] = useState<Tab>('plugins')
  const { plugins, scanning, scanPlugins } = usePluginStore()
  const { selectedTrackId } = useTrackStore()
  const [expanded, setExpanded] = useState<Record<string, boolean>>({ Instruments: true, Effects: true })

  const instruments = plugins.filter(p => p.category === 'Instrument' || p.category === 'Synth')
  const effects = plugins.filter(p => p.category !== 'Instrument' && p.category !== 'Synth')

  return (
    <div style={{
      width: 200,
      minWidth: 200,
      background: '#3A3A3A',
      borderRight: '1px solid rgba(0,0,0,0.4)',
      display: 'flex',
      flexDirection: 'column',
    }}>
      {/* Tabs */}
      <div style={{
        display: 'flex',
        background: '#333',
        borderBottom: '1px solid rgba(0,0,0,0.4)',
      }}>
        {(['plugins', 'files', 'project'] as Tab[]).map(tab => (
          <button
            key={tab}
            onClick={() => setActiveTab(tab)}
            style={{
              flex: 1, padding: '5px 0',
              fontSize: 10, fontWeight: activeTab === tab ? 600 : 400,
              color: activeTab === tab ? '#DDD' : '#888',
              background: activeTab === tab ? '#444' : 'transparent',
              borderBottom: activeTab === tab ? '2px solid #FF8800' : '2px solid transparent',
            }}
          >
            {tab.charAt(0).toUpperCase() + tab.slice(1)}
          </button>
        ))}
      </div>

      {/* Content */}
      <div style={{ flex: 1, overflowY: 'auto', padding: '2px 0' }}>
        {activeTab === 'plugins' && (
          <>
            <div style={{ padding: '4px 6px' }}>
              <button
                onClick={scanPlugins}
                disabled={scanning}
                style={{
                  width: '100%', padding: '4px 0',
                  fontSize: 10, color: '#B0B0B0',
                  background: '#444', border: '1px solid rgba(0,0,0,0.3)',
                  borderRadius: 2, opacity: scanning ? 0.5 : 1,
                }}
              >
                {scanning ? 'Scanning...' : 'Scan Plugins'}
              </button>
            </div>

            {plugins.length === 0 && (
              <div style={{ padding: 16, textAlign: 'center', color: '#777', fontSize: 10 }}>
                Click Scan to find<br />VST3 & CLAP plugins
              </div>
            )}

            {plugins.length > 0 && (
              <>
                <TreeGroup
                  label="Instruments"
                  count={instruments.length}
                  expanded={expanded.Instruments}
                  onToggle={() => setExpanded(p => ({ ...p, Instruments: !p.Instruments }))}
                >
                  {instruments.map(p => (
                    <PluginItem key={p.id} plugin={p} canAdd={!!selectedTrackId} />
                  ))}
                </TreeGroup>
                <TreeGroup
                  label="Effects"
                  count={effects.length}
                  expanded={expanded.Effects}
                  onToggle={() => setExpanded(p => ({ ...p, Effects: !p.Effects }))}
                >
                  {effects.map(p => (
                    <PluginItem key={p.id} plugin={p} canAdd={!!selectedTrackId} />
                  ))}
                </TreeGroup>
              </>
            )}
          </>
        )}

        {activeTab === 'files' && (
          <div style={{ padding: 16, textAlign: 'center', color: '#777', fontSize: 10 }}>
            Drag audio files onto<br />the Playlist to import
          </div>
        )}

        {activeTab === 'project' && (
          <div style={{ padding: 16, textAlign: 'center', color: '#777', fontSize: 10 }}>
            Project files will<br />appear here
          </div>
        )}
      </div>
    </div>
  )
}

function TreeGroup({ label, count, expanded, onToggle, children }: {
  label: string; count: number; expanded: boolean; onToggle: () => void; children: React.ReactNode
}) {
  return (
    <div>
      <div
        onClick={onToggle}
        style={{
          padding: '4px 8px',
          display: 'flex', alignItems: 'center', gap: 4,
          cursor: 'default', fontSize: 11,
        }}
        onMouseEnter={e => { e.currentTarget.style.background = '#4A4A4A' }}
        onMouseLeave={e => { e.currentTarget.style.background = 'transparent' }}
      >
        <span style={{
          fontSize: 8, color: '#AAA',
          transform: expanded ? 'rotate(90deg)' : 'none',
          display: 'inline-block', transition: 'transform 100ms',
        }}>
          {'\u25B6'}
        </span>
        <span style={{ color: '#CCC', fontWeight: 600 }}>{label}</span>
        <span style={{ color: '#777', marginLeft: 'auto' }}>{count}</span>
      </div>
      {expanded && <div style={{ paddingLeft: 8 }}>{children}</div>}
    </div>
  )
}

function PluginItem({ plugin, canAdd }: { plugin: any; canAdd: boolean }) {
  return (
    <div
      style={{
        padding: '3px 8px 3px 16px',
        cursor: canAdd ? 'pointer' : 'default',
        opacity: canAdd ? 1 : 0.5,
      }}
      onClick={async () => {
        if (!canAdd) return
        const { selectedTrackId } = useTrackStore.getState()
        if (!selectedTrackId) return
        await usePluginStore.getState().addToTrack(selectedTrackId, plugin.id)
        useTrackStore.getState().fetchTracks()
      }}
      onMouseEnter={e => { if (canAdd) e.currentTarget.style.background = '#4A4A4A' }}
      onMouseLeave={e => { e.currentTarget.style.background = 'transparent' }}
    >
      <div style={{ fontSize: 10, color: '#CCC' }}>{plugin.name}</div>
      <div style={{ fontSize: 8, color: '#888' }}>{plugin.vendor} · {plugin.format}</div>
    </div>
  )
}
