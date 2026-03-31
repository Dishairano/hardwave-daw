import { hw } from '../../theme'

interface TitleBarProps {
  hintText: string
}

export function TitleBar({ hintText }: TitleBarProps) {
  const menus = ['File', 'Edit', 'Add', 'View', 'Options', 'Tools', 'Help']

  return (
    <div
      data-tauri-drag-region
      style={{
        display: 'flex',
        alignItems: 'center',
        height: 32,
        background: hw.bg,
        borderBottom: `1px solid ${hw.border}`,
        // @ts-ignore
        WebkitAppRegion: 'drag',
        padding: '0 12px',
        gap: 0,
      }}
    >
      {/* Brand mark */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 6,
        marginRight: 16,
        // @ts-ignore
        WebkitAppRegion: 'no-drag',
      }}>
        <div style={{ display: 'flex', gap: 2 }}>
          <div style={{ width: 3, height: 14, background: hw.red, borderRadius: '2px 0 0 2px' }} />
          <div style={{ width: 3, height: 14, background: hw.red }} />
          <div style={{ width: 3, height: 14, background: hw.red, borderRadius: '0 2px 2px 0' }} />
        </div>
        <span style={{ fontSize: 12, fontWeight: 700, color: hw.textPrimary, letterSpacing: 0.5 }}>
          HARDWAVE
        </span>
      </div>

      {/* Menu items */}
      {menus.map(m => (
        <div
          key={m}
          style={{
            padding: '4px 10px',
            fontSize: 12,
            color: hw.textMuted,
            cursor: 'default',
            borderRadius: hw.radius.sm,
            // @ts-ignore
            WebkitAppRegion: 'no-drag',
          }}
          onMouseEnter={e => {
            e.currentTarget.style.background = hw.bgHover
            e.currentTarget.style.color = hw.textSecondary
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
        maxWidth: 350,
      }}>
        {hintText}
      </div>
    </div>
  )
}
