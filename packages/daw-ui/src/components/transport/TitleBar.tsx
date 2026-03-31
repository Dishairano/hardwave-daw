export function TitleBar() {
  return (
    <div
      data-tauri-drag-region
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        height: 32,
        background: '#0e0e10',
        borderBottom: '1px solid rgba(255,255,255,0.04)',
        // @ts-ignore — Tauri drag region
        WebkitAppRegion: 'drag',
      }}
    >
      <span style={{ fontSize: 11, fontWeight: 700, color: '#555', letterSpacing: 2 }}>
        HARDWAVE DAW
      </span>
    </div>
  )
}
