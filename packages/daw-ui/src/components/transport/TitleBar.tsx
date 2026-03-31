export function TitleBar() {
  const menus = ['FILE', 'EDIT', 'ADD', 'PATTERNS', 'VIEW', 'OPTIONS', 'TOOLS', 'HELP']

  return (
    <div
      data-tauri-drag-region
      style={{
        display: 'flex',
        alignItems: 'center',
        height: 24,
        background: '#1E1E1E',
        borderBottom: '1px solid #111',
        // @ts-ignore
        WebkitAppRegion: 'drag',
        paddingLeft: 8,
      }}
    >
      {/* Logo */}
      <div style={{
        fontSize: 10,
        fontWeight: 800,
        color: '#FF6B00',
        letterSpacing: 1.5,
        marginRight: 16,
        // @ts-ignore
        WebkitAppRegion: 'no-drag',
      }}>
        HARDWAVE
      </div>

      {/* Menu items */}
      {menus.map(m => (
        <div
          key={m}
          style={{
            padding: '0 8px',
            height: '100%',
            display: 'flex',
            alignItems: 'center',
            fontSize: 10,
            color: '#999',
            cursor: 'pointer',
            // @ts-ignore
            WebkitAppRegion: 'no-drag',
          }}
          onMouseEnter={e => { e.currentTarget.style.background = '#333'; e.currentTarget.style.color = '#fff' }}
          onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = '#999' }}
        >
          {m}
        </div>
      ))}

      <div style={{ flex: 1 }} />

      {/* Window title */}
      <span style={{ fontSize: 10, color: '#555', marginRight: 8 }}>
        Hardwave DAW v0.1.3
      </span>
    </div>
  )
}
