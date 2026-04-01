import { hw } from '../../theme'

interface TitleBarProps {
  hintText: string
}

export function TitleBar({ hintText }: TitleBarProps) {
  const menus = ['FILE', 'EDIT', 'ADD', 'PATTERNS', 'VIEW', 'OPTIONS', 'TOOLS', 'HELP']

  return (
    <div
      data-tauri-drag-region
      style={{
        display: 'flex',
        alignItems: 'center',
        height: 26,
        background: '#3C3C3C',
        borderBottom: '1px solid rgba(0,0,0,0.5)',
        // @ts-ignore
        WebkitAppRegion: 'drag',
        padding: '0 6px',
      }}
    >
      {/* FL Logo placeholder */}
      <div style={{
        width: 18, height: 18, marginRight: 6,
        background: 'linear-gradient(135deg, #FF8800, #FF6600)',
        borderRadius: 3,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        // @ts-ignore
        WebkitAppRegion: 'no-drag',
      }}>
        <span style={{ fontSize: 10, fontWeight: 900, color: '#FFF' }}>H</span>
      </div>

      {/* Menu items */}
      {menus.map(m => (
        <div
          key={m}
          style={{
            padding: '3px 8px',
            fontSize: 11,
            color: '#B0B0B0',
            cursor: 'default',
            // @ts-ignore
            WebkitAppRegion: 'no-drag',
          }}
          onMouseEnter={e => {
            e.currentTarget.style.background = '#555555'
            e.currentTarget.style.color = '#FFFFFF'
          }}
          onMouseLeave={e => {
            e.currentTarget.style.background = 'transparent'
            e.currentTarget.style.color = '#B0B0B0'
          }}
        >
          {m}
        </div>
      ))}

      <div style={{ flex: 1 }} />

      {/* Hint text (FL hint bar) */}
      <div style={{
        fontSize: 11,
        color: '#888888',
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        whiteSpace: 'nowrap',
        maxWidth: 400,
        marginRight: 4,
      }}>
        {hintText || 'Hardwave DAW'}
      </div>

      {/* Window controls */}
      <div style={{ display: 'flex', gap: 0, marginLeft: 8,
        // @ts-ignore
        WebkitAppRegion: 'no-drag',
      }}>
        <WinBtn label="\u2012" />
        <WinBtn label="\u25A1" />
        <WinBtn label="\u00D7" isClose />
      </div>
    </div>
  )
}

function WinBtn({ label, isClose }: { label: string; isClose?: boolean }) {
  return (
    <div
      style={{
        width: 28, height: 22,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        fontSize: 12, color: '#999',
        cursor: 'default',
      }}
      onMouseEnter={e => {
        e.currentTarget.style.background = isClose ? '#C42B1C' : '#555'
        e.currentTarget.style.color = '#FFF'
      }}
      onMouseLeave={e => {
        e.currentTarget.style.background = 'transparent'
        e.currentTarget.style.color = '#999'
      }}
    >
      {label}
    </div>
  )
}
