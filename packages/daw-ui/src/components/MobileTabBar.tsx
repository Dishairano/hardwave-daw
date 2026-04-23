import { hw } from '../theme'

export type MobilePanel = 'browser' | 'playlist' | 'channelRack' | 'pianoRoll' | 'mixer'

interface Props {
  active: MobilePanel
  onSelect: (panel: MobilePanel) => void
}

const TABS: Array<{ id: MobilePanel; label: string; icon: string }> = [
  { id: 'browser',     label: 'Browse',  icon: '📁' },
  { id: 'playlist',    label: 'Arrange', icon: '▤' },
  { id: 'channelRack', label: 'Rack',    icon: '▦' },
  { id: 'pianoRoll',   label: 'Piano',   icon: '♪' },
  { id: 'mixer',       label: 'Mixer',   icon: '▥' },
]

export function MobileTabBar({ active, onSelect }: Props) {
  return (
    <div
      role="tablist"
      style={{
        display: 'flex',
        flexShrink: 0,
        height: 56,
        background: hw.bgToolbarGrad,
        borderTop: `1px solid ${hw.borderDark}`,
        paddingBottom: 'env(safe-area-inset-bottom)',
      }}
    >
      {TABS.map(tab => {
        const isActive = active === tab.id
        return (
          <button
            key={tab.id}
            role="tab"
            aria-selected={isActive}
            onClick={() => onSelect(tab.id)}
            style={{
              flex: 1,
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              justifyContent: 'center',
              gap: 2,
              background: 'transparent',
              border: 'none',
              color: isActive ? hw.accent : hw.textMuted,
              fontSize: 10,
              fontWeight: 600,
              cursor: 'pointer',
              padding: 4,
              borderTop: isActive ? `2px solid ${hw.accent}` : '2px solid transparent',
              transition: 'color 120ms ease',
              touchAction: 'manipulation',
            }}
          >
            <span style={{ fontSize: 18, lineHeight: 1 }}>{tab.icon}</span>
            <span style={{ letterSpacing: 0.2 }}>{tab.label}</span>
          </button>
        )
      })}
    </div>
  )
}
