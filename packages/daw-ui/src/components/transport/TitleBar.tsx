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
        height: 22,
        background: hw.bgDeep,
        borderBottom: `1px solid ${hw.borderDark}`,
        // @ts-ignore
        WebkitAppRegion: 'drag',
        padding: '0 6px',
      }}
    >
      {/* Menu items — compact, FL style */}
      {menus.map(m => (
        <div
          key={m}
          style={{
            padding: '2px 7px',
            fontSize: 11,
            color: hw.textMuted,
            cursor: 'default',
            // @ts-ignore
            WebkitAppRegion: 'no-drag',
          }}
          onMouseEnter={e => {
            e.currentTarget.style.background = hw.bgHover
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

      {/* Hint text (FL hint bar) */}
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
    </div>
  )
}
