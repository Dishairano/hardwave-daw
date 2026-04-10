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
        height: 28,
        background: hw.bg,
        borderBottom: `1px solid ${hw.border}`,
        // @ts-ignore
        WebkitAppRegion: 'drag',
        padding: '0 8px',
      }}
    >
      {/* Hardwave Logo */}
      <div style={{
        width: 20, height: 20, marginRight: 8,
        background: 'linear-gradient(135deg, #9B6DFF, #7B5AC0)',
        borderRadius: 6,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        boxShadow: '0 0 12px rgba(155, 109, 255, 0.2)',
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
            padding: '4px 8px',
            fontSize: 11,
            color: hw.textMuted,
            cursor: 'default',
            borderRadius: hw.radius.sm,
            transition: 'color 0.1s, background 0.1s',
            // @ts-ignore
            WebkitAppRegion: 'no-drag',
          }}
          onMouseEnter={e => {
            e.currentTarget.style.background = hw.bgElevated
            e.currentTarget.style.color = hw.textPrimary
          }}
          onMouseLeave={e => {
            e.currentTarget.style.background = 'transparent'
            e.currentTarget.style.color = hw.textMuted
          }}
        >
          {m}
        </div>
      ))}

      <div style={{ flex: 1 }} />

      {/* Hint text */}
      <div style={{
        fontSize: 11,
        color: hw.textFaint,
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
        width: 30, height: 24,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        fontSize: 12, color: hw.textMuted,
        cursor: 'default',
        borderRadius: hw.radius.sm,
      }}
      onMouseEnter={e => {
        e.currentTarget.style.background = isClose ? '#C42B1C' : hw.bgElevated
        e.currentTarget.style.color = '#FFF'
      }}
      onMouseLeave={e => {
        e.currentTarget.style.background = 'transparent'
        e.currentTarget.style.color = hw.textMuted
      }}
    >
      {label}
    </div>
  )
}
