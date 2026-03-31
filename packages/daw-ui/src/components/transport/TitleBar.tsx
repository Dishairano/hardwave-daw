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
        height: 20,
        background: '#2C2C2C',
        borderBottom: '1px solid #1A1A1A',
        // @ts-ignore
        WebkitAppRegion: 'drag',
        fontSize: 11,
      }}
    >
      {/* Menu items — FL Studio puts these directly in the title bar */}
      {menus.map(m => (
        <div
          key={m}
          style={{
            padding: '0 6px',
            height: '100%',
            display: 'flex',
            alignItems: 'center',
            fontSize: 11,
            color: '#999',
            cursor: 'default',
            // @ts-ignore
            WebkitAppRegion: 'no-drag',
          }}
          onMouseEnter={e => {
            e.currentTarget.style.background = '#454545'
            e.currentTarget.style.color = '#DDD'
          }}
          onMouseLeave={e => {
            e.currentTarget.style.background = 'transparent'
            e.currentTarget.style.color = '#999'
          }}
        >
          {m}
        </div>
      ))}

      {/* Spacer */}
      <div style={{ flex: 1 }} />

      {/* Hint text — FL shows contextual help here */}
      <div style={{
        fontSize: 11,
        color: '#666',
        marginRight: 8,
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        whiteSpace: 'nowrap',
        maxWidth: 400,
      }}>
        {hintText || 'Hardwave DAW v0.1.5'}
      </div>
    </div>
  )
}
