import { useState, useEffect, useRef } from 'react'
import { hw } from '../../theme'

interface MenuItem {
  label: string
  shortcut?: string
  action?: () => void
  separator?: boolean
  disabled?: boolean
}

interface MenuDef {
  label: string
  items: MenuItem[]
}

interface TitleBarProps {
  hintText: string
  onNewProject: () => void
  onSaveProject: () => void
  onSaveProjectAs: () => void
  onOpenProject: () => void
  onUndo: () => void
  onRedo: () => void
  onToggleBrowser: () => void
  onTogglePlaylist: () => void
  onToggleChannelRack: () => void
  onTogglePianoRoll: () => void
  onToggleMixer: () => void
  onToggleRoadmap: () => void
  onOpenAudioSettings: () => void
  showBrowser: boolean
  showPlaylist: boolean
  showChannelRack: boolean
  showPianoRoll: boolean
  showMixer: boolean
}

export function TitleBar({
  hintText,
  onNewProject, onSaveProject, onSaveProjectAs, onOpenProject,
  onUndo, onRedo,
  onToggleBrowser, onTogglePlaylist, onToggleChannelRack, onTogglePianoRoll, onToggleMixer, onToggleRoadmap, onOpenAudioSettings,
  showBrowser, showPlaylist, showChannelRack, showPianoRoll, showMixer,
}: TitleBarProps) {
  const [openMenu, setOpenMenu] = useState<number | null>(null)
  const barRef = useRef<HTMLDivElement>(null)

  const menus: MenuDef[] = [
    {
      label: 'FILE',
      items: [
        { label: 'New project', shortcut: 'Ctrl+N', action: onNewProject },
        { label: 'Open project...', shortcut: 'Ctrl+O', action: onOpenProject },
        { separator: true, label: '' },
        { label: 'Save', shortcut: 'Ctrl+S', action: onSaveProject },
        { label: 'Save as...', shortcut: 'Ctrl+Shift+S', action: onSaveProjectAs },
        { separator: true, label: '' },
        { label: 'Exit', action: () => windowClose() },
      ],
    },
    {
      label: 'EDIT',
      items: [
        { label: 'Undo', shortcut: 'Ctrl+Z', action: onUndo, disabled: true },
        { label: 'Redo', shortcut: 'Ctrl+Y', action: onRedo, disabled: true },
      ],
    },
    {
      label: 'ADD',
      items: [
        { label: 'Audio track', disabled: true },
        { label: 'Instrument track', disabled: true },
        { separator: true, label: '' },
        { label: 'Plugin...', disabled: true },
      ],
    },
    {
      label: 'PATTERNS',
      items: [
        { label: 'New pattern', disabled: true },
        { label: 'Clone pattern', disabled: true },
        { label: 'Delete pattern', disabled: true },
      ],
    },
    {
      label: 'VIEW',
      items: [
        { label: `${showBrowser ? '✓ ' : '   '}Browser`, shortcut: 'F8', action: onToggleBrowser },
        { label: `${showPlaylist ? '✓ ' : '   '}Playlist`, shortcut: 'F5', action: onTogglePlaylist },
        { label: `${showChannelRack ? '✓ ' : '   '}Channel Rack`, shortcut: 'F6', action: onToggleChannelRack },
        { label: `${showPianoRoll ? '✓ ' : '   '}Piano Roll`, shortcut: 'F7', action: onTogglePianoRoll },
        { label: `${showMixer ? '✓ ' : '   '}Mixer`, shortcut: 'F9', action: onToggleMixer },
      ],
    },
    {
      label: 'OPTIONS',
      items: [
        { label: 'Audio settings...', action: onOpenAudioSettings },
        { label: 'MIDI settings...', disabled: true },
      ],
    },
    {
      label: 'TOOLS',
      items: [
        { label: 'Macros...', disabled: true },
        { label: 'External editor...', disabled: true },
      ],
    },
    {
      label: 'HELP',
      items: [
        { label: 'Roadmap', action: onToggleRoadmap },
        { separator: true, label: '' },
        { label: 'About Hardwave DAW', disabled: true },
      ],
    },
  ]

  // Close menu when clicking outside
  useEffect(() => {
    if (openMenu === null) return
    const handler = (e: MouseEvent) => {
      if (barRef.current && !barRef.current.contains(e.target as Node)) {
        setOpenMenu(null)
      }
    }
    window.addEventListener('mousedown', handler)
    return () => window.removeEventListener('mousedown', handler)
  }, [openMenu])

  // Close menu on Escape
  useEffect(() => {
    if (openMenu === null) return
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpenMenu(null)
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [openMenu])

  return (
    <div
      ref={barRef}
      data-tauri-drag-region
      style={{
        display: 'flex',
        alignItems: 'center',
        height: 28,
        background: 'rgba(255,255,255,0.02)',
        backdropFilter: hw.blur.md,
        borderBottom: `1px solid ${hw.border}`,
        // @ts-ignore
        WebkitAppRegion: 'drag',
        padding: '0 8px',
        position: 'relative',
        zIndex: 100,
      }}
    >
      {/* Hardwave Logo — shield with checkmark, matches mockup */}
      <div style={{
        width: 22, height: 22, marginRight: 10,
        background: 'linear-gradient(145deg, #1c1c24 0%, #0a0a0e 100%)',
        border: `1px solid ${hw.borderLight}`,
        borderRadius: 6,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        position: 'relative',
        overflow: 'hidden',
        // @ts-ignore
        WebkitAppRegion: 'no-drag',
      }}>
        <div style={{
          position: 'absolute', inset: 1, borderRadius: 5,
          background: 'radial-gradient(circle at 30% 20%, rgba(239,68,68,0.25), transparent 60%)',
          pointerEvents: 'none',
        }} />
        <svg width={12} height={12} viewBox="0 0 24 24" fill="none"
          stroke={hw.accentLight} strokeWidth={2.4} strokeLinecap="round" strokeLinejoin="round"
          style={{ position: 'relative', zIndex: 1 }}>
          <path d="M12 2 L4 6 v6 c0 4.5 3.5 8.5 8 10 c4.5-1.5 8-5.5 8-10 V6 z" />
          <path d="M9 12 l2 2 l4-4" />
        </svg>
      </div>

      {/* Menu items */}
      {menus.map((menu, idx) => (
        <div
          key={menu.label}
          style={{ position: 'relative' }}
        >
          <div
            style={{
              padding: '4px 9px',
              fontFamily: hw.font.ui,
              fontSize: 10.5,
              fontWeight: 600,
              letterSpacing: hw.tracking.eyebrow,
              color: openMenu === idx ? hw.textPrimary : hw.textMuted,
              cursor: 'default',
              borderRadius: hw.radius.sm,
              background: openMenu === idx ? 'rgba(255,255,255,0.08)' : 'transparent',
              transition: 'color 0.15s, background 0.15s',
              // @ts-ignore
              WebkitAppRegion: 'no-drag',
            }}
            onMouseDown={(e) => {
              e.stopPropagation()
              setOpenMenu(openMenu === idx ? null : idx)
            }}
            onMouseEnter={(e) => {
              if (openMenu !== null && openMenu !== idx) {
                setOpenMenu(idx)
              }
              if (openMenu !== idx) {
                e.currentTarget.style.background = 'rgba(255,255,255,0.06)'
                e.currentTarget.style.color = hw.textPrimary
              }
            }}
            onMouseLeave={(e) => {
              if (openMenu !== idx) {
                e.currentTarget.style.background = 'transparent'
                e.currentTarget.style.color = hw.textMuted
              }
            }}
          >
            {menu.label}
          </div>

          {/* Dropdown */}
          {openMenu === idx && (
            <div style={{
              position: 'absolute',
              top: '100%',
              left: 0,
              marginTop: 2,
              minWidth: 200,
              background: 'rgba(18,18,22,0.98)',
              backdropFilter: 'blur(16px)',
              border: `1px solid ${hw.borderLight}`,
              borderRadius: hw.radius.md,
              padding: '4px 0',
              boxShadow: '0 12px 40px rgba(0,0,0,0.6), 0 0 1px rgba(255,255,255,0.05)',
              zIndex: 200,
            }}>
              {menu.items.map((item, i) =>
                item.separator ? (
                  <div key={i} style={{
                    height: 1,
                    margin: '4px 8px',
                    background: hw.border,
                  }} />
                ) : (
                  <div
                    key={i}
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      justifyContent: 'space-between',
                      padding: '6px 12px',
                      fontFamily: hw.font.ui,
                      fontSize: 12,
                      fontWeight: 500,
                      color: item.disabled ? hw.textFaint : hw.textSecondary,
                      cursor: item.disabled ? 'default' : 'pointer',
                      transition: 'background 0.1s, color 0.1s',
                    }}
                    onMouseEnter={e => {
                      if (!item.disabled) {
                        e.currentTarget.style.background = hw.accentDim
                        e.currentTarget.style.color = hw.textPrimary
                      }
                    }}
                    onMouseLeave={e => {
                      e.currentTarget.style.background = 'transparent'
                      e.currentTarget.style.color = item.disabled ? hw.textFaint : hw.textSecondary
                    }}
                    onClick={() => {
                      if (item.disabled || !item.action) return
                      item.action()
                      setOpenMenu(null)
                    }}
                  >
                    <span>{item.label}</span>
                    {item.shortcut && (
                      <span style={{
                        fontFamily: hw.font.mono,
                        fontSize: 10,
                        fontWeight: 400,
                        color: hw.textFaint,
                        marginLeft: 24,
                        letterSpacing: hw.tracking.wide,
                      }}>
                        {item.shortcut}
                      </span>
                    )}
                  </div>
                )
              )}
            </div>
          )}
        </div>
      ))}

      <div style={{ flex: 1 }} />

      {/* Hint text */}
      <div style={{
        fontFamily: hw.font.ui,
        fontSize: 11,
        fontWeight: 500,
        color: hw.textFaint,
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        whiteSpace: 'nowrap',
        maxWidth: 400,
        marginRight: 4,
        letterSpacing: '0.01em',
      }}>
        {hintText || 'Hardwave DAW'}
      </div>

      {/* Window controls */}
      <div style={{ display: 'flex', gap: 0, marginLeft: 8,
        // @ts-ignore
        WebkitAppRegion: 'no-drag',
      }}>
        <WinBtn label={'\u2012'} onClick={windowMinimize} />
        <WinBtn label={'\u25A1'} onClick={windowToggleMaximize} />
        <WinBtn label={'\u00D7'} isClose onClick={windowClose} />
      </div>
    </div>
  )
}

function WinBtn({ label, isClose, onClick }: { label: string; isClose?: boolean; onClick: () => void }) {
  return (
    <div
      style={{
        width: 30, height: 24,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        fontSize: 12, color: hw.textMuted,
        cursor: 'default',
        borderRadius: hw.radius.sm,
        transition: 'background 0.15s, color 0.15s',
      }}
      onClick={onClick}
      onMouseEnter={e => {
        e.currentTarget.style.background = isClose ? '#C42B1C' : 'rgba(255,255,255,0.08)'
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

async function windowMinimize() {
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    await getCurrentWindow().minimize()
  } catch {}
}

async function windowToggleMaximize() {
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    await getCurrentWindow().toggleMaximize()
  } catch {}
}

async function windowClose() {
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    await getCurrentWindow().close()
  } catch {}
}
