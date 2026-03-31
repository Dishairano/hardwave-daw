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

  // Group plugins by category
  const instruments = plugins.filter(p => p.category === 'Instrument' || p.category === 'Synth')
  const effects = plugins.filter(p => p.category !== 'Instrument' && p.category !== 'Synth')

  return (
    <div style={{
      width: 195,
      minWidth: 195,
      background: hw.bgDark,
      borderRight: `1px solid ${hw.borderDark}`,
      display: 'flex',
      flexDirection: 'column',
    }}>
      {/* Tabs */}
      <div style={{
        display: 'flex',
        background: hw.bgDeep,
        borderBottom: `1px solid ${hw.borderDark}`,
      }}>
        {(['plugins', 'files', 'project'] as Tab[]).map(tab => (
          <button
            key={tab}
            onClick={() => setActiveTab(tab)}
            style={{
              flex: 1, padding: '5px 0',
              fontSize: 10, fontWeight: 500,
              color: activeTab === tab ? hw.textPrimary : hw.textFaint,
              borderBottom: activeTab === tab ? `2px solid ${hw.purple}` : '2px solid transparent',
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
                  fontSize: 10, color: hw.textMuted,
                  background: hw.bgPanel, border: `1px solid ${hw.border}`,
                  borderRadius: 3, opacity: scanning ? 0.5 : 1,
                }}
              >
                {scanning ? 'Scanning...' : 'Scan Plugins'}
              </button>
            </div>

            {plugins.length === 0 && (
              <div style={{ padding: 16, textAlign: 'center', color: hw.textFaint, fontSize: 10 }}>
                Click Scan to find<br />VST3 & CLAP plugins
              </div>
            )}

            {/* Tree-style groups */}
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
          <div style={{ padding: 16, textAlign: 'center', color: hw.textFaint, fontSize: 10 }}>
            Drag audio files onto<br />the Playlist to import
          </div>
        )}

        {activeTab === 'project' && (
          <div style={{ padding: 16, textAlign: 'center', color: hw.textFaint, fontSize: 10 }}>
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
          padding: '3px 8px',
          display: 'flex', alignItems: 'center', gap: 4,
          cursor: 'default', fontSize: 10,
        }}
        onMouseEnter={e => { e.currentTarget.style.background = hw.bgHover }}
        onMouseLeave={e => { e.currentTarget.style.background = 'transparent' }}
      >
        <span style={{
          fontSize: 8, color: hw.textFaint,
          transform: expanded ? 'rotate(90deg)' : 'none',
          display: 'inline-block', transition: 'transform 100ms',
        }}>
          {'\u25B6'}
        </span>
        <span style={{ color: hw.textSecondary, fontWeight: 600 }}>{label}</span>
        <span style={{ color: hw.textFaint, marginLeft: 'auto' }}>{count}</span>
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
      onMouseEnter={e => { if (canAdd) e.currentTarget.style.background = hw.bgHover }}
      onMouseLeave={e => { e.currentTarget.style.background = 'transparent' }}
    >
      <div style={{ fontSize: 10, color: hw.textSecondary }}>{plugin.name}</div>
      <div style={{ fontSize: 8, color: hw.textFaint }}>{plugin.vendor} · {plugin.format}</div>
    </div>
  )
}
