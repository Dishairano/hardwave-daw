import { useState, useEffect, useRef } from 'react'
import { hw } from '../../theme'
import { usePatternStore } from '../../stores/patternStore'
import { useProjectStore } from '../../stores/projectStore'
import { useUiPreferencesStore, UI_SCALE_OPTIONS, type UiScale } from '../../stores/uiPreferencesStore'
import { useTrackTemplateStore } from '../../stores/trackTemplateStore'
import { useIsMobile } from '../../hooks/useIsMobile'

function openExternal(url: string) {
  window.open(url, '_blank', 'noopener,noreferrer')
}

interface MenuItem {
  label: string
  shortcut?: string
  action?: () => void
  separator?: boolean
  disabled?: boolean
  submenu?: MenuItem[]
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
  onCut: () => void
  onCopy: () => void
  onPaste: () => void
  onDuplicate: () => void
  onSelectAll: () => void
  onAddAudioTrack: () => void
  onAddInstrumentTrack: () => void
  onAddAutomationTrack: () => void
  onApplyTrackTemplate: (templateId: string) => void
  onManageTrackTemplates: () => void
  onToggleBrowser: () => void
  onTogglePlaylist: () => void
  onToggleChannelRack: () => void
  onTogglePianoRoll: () => void
  onToggleMixer: () => void
  onToggleRoadmap: () => void
  onOpenAudioSettings: () => void
  onOpenThemePicker: () => void
  onOpenLoudness: () => void
  onOpenOscilloscope: () => void
  onOpenSpectrum: () => void
  onOpenMidiMappings: () => void
  onOpenTempoMap: () => void
  pdcEnabled: boolean
  onTogglePdc: () => void
  onCheckForUpdates: () => void
  onToggleAbout: () => void
  onToggleShortcuts: () => void
  onToggleHelp: () => void
  onOpenHistory: () => void
  onExportAudio: () => void
  onSaveAsTemplate: () => void
  onAutoCrossfade: () => void
  recentProjects: string[]
  onOpenRecentProject: (path: string) => void
  onClearRecentProjects: () => void
  showBrowser: boolean
  showPlaylist: boolean
  showChannelRack: boolean
  showPianoRoll: boolean
  showMixer: boolean
}

export function TitleBar(props: TitleBarProps) {
  const {
    hintText,
    onNewProject, onSaveProject, onSaveProjectAs, onOpenProject,
    onUndo, onRedo,
    onCut, onCopy, onPaste, onDuplicate, onSelectAll,
    onAddAudioTrack, onAddInstrumentTrack, onAddAutomationTrack,
    onApplyTrackTemplate, onManageTrackTemplates,
    onToggleBrowser, onTogglePlaylist, onToggleChannelRack, onTogglePianoRoll, onToggleMixer,
    onToggleRoadmap, onOpenAudioSettings, onOpenThemePicker, onOpenLoudness, onOpenOscilloscope, onOpenSpectrum, onOpenMidiMappings, onOpenTempoMap, pdcEnabled, onTogglePdc, onCheckForUpdates, onToggleAbout, onToggleShortcuts, onToggleHelp, onOpenHistory, onExportAudio,
    onSaveAsTemplate, onAutoCrossfade,
    recentProjects, onOpenRecentProject, onClearRecentProjects,
    showBrowser, showPlaylist, showChannelRack, showPianoRoll, showMixer,
  } = props
  const [openMenu, setOpenMenu] = useState<number | null>(null)
  const [openSubmenu, setOpenSubmenu] = useState<number | null>(null)
  const barRef = useRef<HTMLDivElement>(null)
  const isMobile = useIsMobile()
  const addPattern = usePatternStore(s => s.addPattern)
  const clonePattern = usePatternStore(s => s.clonePattern)
  const deletePattern = usePatternStore(s => s.deletePattern)
  const patternCount = usePatternStore(s => s.patterns.length)
  const uiScaleMode = useUiPreferencesStore(s => s.mode)
  const effectiveScale = useUiPreferencesStore(s => s.effectiveScale)
  const setUiScaleMode = useUiPreferencesStore(s => s.setUiScaleMode)
  const trackTemplates = useTrackTemplateStore(s => s.templates)

  const uiScaleItems: MenuItem[] = [
    {
      label: `${uiScaleMode === 'auto' ? '✓ ' : '   '}Auto (detected: ${effectiveScale}%)`,
      action: () => setUiScaleMode('auto'),
    },
    { separator: true, label: '' },
    ...UI_SCALE_OPTIONS.map(scale => ({
      label: `${uiScaleMode === scale ? '✓ ' : '   '}${scale}%`,
      action: () => setUiScaleMode(scale as UiScale),
    })),
  ]

  const recentItems: MenuItem[] = recentProjects.length === 0
    ? [{ label: '(none)', disabled: true }]
    : [
        ...recentProjects.slice(0, 10).map((p) => ({
          label: shortenPath(p),
          action: () => onOpenRecentProject(p),
        })),
        { separator: true, label: '' },
        { label: 'Clear recent', action: onClearRecentProjects },
      ]

  const menus: MenuDef[] = [
    {
      label: 'FILE',
      items: [
        { label: 'New project', shortcut: 'Ctrl+N', action: onNewProject },
        { label: 'Open project...', shortcut: 'Ctrl+O', action: onOpenProject },
        { label: 'Recent projects', submenu: recentItems },
        { separator: true, label: '' },
        { label: 'Save', shortcut: 'Ctrl+S', action: onSaveProject },
        { label: 'Save as...', shortcut: 'Ctrl+Shift+S', action: onSaveProjectAs },
        { label: 'Save as template...', action: onSaveAsTemplate },
        { separator: true, label: '' },
        { label: 'Export audio...', action: onExportAudio },
        { separator: true, label: '' },
        { label: 'Exit', action: () => windowClose() },
      ],
    },
    {
      label: 'EDIT',
      items: [
        { label: 'Undo', shortcut: 'Ctrl+Z', action: onUndo },
        { label: 'Redo', shortcut: 'Ctrl+Y', action: onRedo },
        { label: 'History...', action: onOpenHistory },
        { separator: true, label: '' },
        { label: 'Cut', shortcut: 'Ctrl+X', action: onCut },
        { label: 'Copy', shortcut: 'Ctrl+C', action: onCopy },
        { label: 'Paste', shortcut: 'Ctrl+V', action: onPaste },
        { label: 'Duplicate', shortcut: 'Ctrl+D', action: onDuplicate },
        { separator: true, label: '' },
        { label: 'Select all', shortcut: 'Ctrl+A', action: onSelectAll },
        { separator: true, label: '' },
        { label: 'Auto-crossfade overlaps', action: onAutoCrossfade },
      ],
    },
    {
      label: 'ADD',
      items: [
        { label: 'Audio track', action: onAddAudioTrack },
        { label: 'Instrument track', action: onAddInstrumentTrack },
        { label: 'Automation track', action: onAddAutomationTrack },
        { separator: true, label: '' },
        {
          label: 'Track from template',
          submenu: trackTemplates.length === 0
            ? [{ label: '(no templates yet)', disabled: true }]
            : [
                ...trackTemplates.map(t => ({
                  label: `${t.name} — ${t.kind}`,
                  action: () => onApplyTrackTemplate(t.id),
                })),
                { separator: true, label: '' },
                { label: 'Manage templates…', action: onManageTrackTemplates },
              ],
        },
        { separator: true, label: '' },
        { label: 'Plugin...', disabled: true },
      ],
    },
    {
      label: 'PATTERNS',
      items: [
        { label: 'New pattern', action: addPattern },
        { label: 'Clone pattern', action: clonePattern },
        { label: 'Delete pattern', action: deletePattern, disabled: patternCount <= 1 },
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
        { label: 'Tempo map...', action: onOpenTempoMap },
        { label: `${pdcEnabled ? '✓ ' : '   '}Plugin delay compensation`, action: onTogglePdc },
        { separator: true, label: '' },
        { label: 'Theme...', action: onOpenThemePicker },
        { label: 'UI scale', submenu: uiScaleItems },
      ],
    },
    {
      label: 'TOOLS',
      items: [
        { label: 'Loudness meter...', action: onOpenLoudness },
        { label: 'Oscilloscope...', action: onOpenOscilloscope },
        { label: 'Spectrum analyzer...', action: onOpenSpectrum },
        { label: 'MIDI mappings...', action: onOpenMidiMappings },
        { separator: true, label: '' },
        { label: 'Macros...', disabled: true },
        { label: 'External editor...', disabled: true },
      ],
    },
    {
      label: 'HELP',
      items: [
        { label: 'Help topics', shortcut: 'F1', action: onToggleHelp },
        { label: 'Keyboard shortcuts', shortcut: 'Shift+F1', action: onToggleShortcuts },
        { label: 'Roadmap', action: onToggleRoadmap },
        { label: 'Check for updates...', action: onCheckForUpdates },
        { separator: true, label: '' },
        { label: 'Online user manual', action: () => openExternal('https://github.com/Dishairano/hardwave-daw/wiki') },
        { label: 'Video tutorials', submenu: [
          { label: 'Getting started', action: () => openExternal('https://www.youtube.com/@hardwavestudios') },
          { label: 'Piano Roll basics', action: () => openExternal('https://www.youtube.com/@hardwavestudios') },
          { label: 'Mixing workflow', action: () => openExternal('https://www.youtube.com/@hardwavestudios') },
          { label: 'Sampling & slicing', action: () => openExternal('https://www.youtube.com/@hardwavestudios') },
          { separator: true, label: '' },
          { label: 'All tutorials →', action: () => openExternal('https://www.youtube.com/@hardwavestudios') },
        ]},
        { label: 'Release notes', action: () => openExternal('https://github.com/Dishairano/hardwave-daw/releases') },
        { label: 'Report an issue', action: () => openExternal('https://github.com/Dishairano/hardwave-daw/issues') },
        { separator: true, label: '' },
        { label: 'About Hardwave DAW', action: onToggleAbout },
      ],
    },
  ]

  useEffect(() => {
    if (openMenu === null) return
    const handler = (e: MouseEvent) => {
      if (barRef.current && !barRef.current.contains(e.target as Node)) {
        setOpenMenu(null)
        setOpenSubmenu(null)
      }
    }
    window.addEventListener('mousedown', handler)
    return () => window.removeEventListener('mousedown', handler)
  }, [openMenu])

  useEffect(() => {
    if (openMenu === null) return
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setOpenMenu(null)
        setOpenSubmenu(null)
      }
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
        height: isMobile ? 36 : 28,
        flexShrink: 0,
        background: 'rgba(255,255,255,0.02)',
        backdropFilter: hw.blur.md,
        borderBottom: `1px solid ${hw.border}`,
        // @ts-ignore
        WebkitAppRegion: 'drag',
        padding: '0 8px',
        position: 'relative',
        zIndex: 100,
        overflowX: isMobile ? 'auto' : 'visible',
        overflowY: 'hidden',
        WebkitOverflowScrolling: 'touch',
      }}
    >
      {/* Hardwave shield logo (matches mockup + splash + About dialog) */}
      <div style={{
        width: 22, height: 22, marginRight: 10,
        background: 'linear-gradient(145deg, #1c1c24 0%, #0a0a0e 100%)',
        border: `1px solid ${hw.borderLight}`,
        borderRadius: 6,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        position: 'relative', overflow: 'hidden',
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

      {menus.map((menu, idx) => (
        <div key={menu.label} style={{ position: 'relative' }}>
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
              setOpenSubmenu(null)
            }}
            onMouseEnter={(e) => {
              if (openMenu !== null && openMenu !== idx) {
                setOpenMenu(idx)
                setOpenSubmenu(null)
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

          {openMenu === idx && (
            <div style={dropdownStyle}>
              {menu.items.map((item, i) =>
                item.separator ? (
                  <div key={i} style={{ height: 1, margin: '4px 8px', background: hw.border }} />
                ) : (
                  <MenuRow
                    key={i}
                    item={item}
                    isSubmenuOpen={openSubmenu === i}
                    onEnterSubmenu={() => item.submenu && setOpenSubmenu(i)}
                    onLeaveSubmenu={() => setOpenSubmenu((s) => (s === i ? null : s))}
                    onClickItem={() => {
                      if (item.disabled) return
                      if (item.submenu) return
                      item.action?.()
                      setOpenMenu(null)
                      setOpenSubmenu(null)
                    }}
                    onClickSubmenuItem={() => {
                      setOpenMenu(null)
                      setOpenSubmenu(null)
                    }}
                  />
                )
              )}
            </div>
          )}
        </div>
      ))}

      <div style={{ flex: 1 }} />

      <ProjectTitleDisplay hintText={hintText} />

      {!isMobile && (
        <div style={{ display: 'flex', gap: 0, marginLeft: 8,
          // @ts-ignore
          WebkitAppRegion: 'no-drag',
        }}>
          <WinBtn label={'‒'} onClick={windowMinimize} />
          <WinBtn label={'□'} onClick={windowToggleMaximize} />
          <WinBtn label={'×'} isClose onClick={windowClose} />
        </div>
      )}
    </div>
  )
}

function ProjectTitleDisplay({ hintText }: { hintText: string }) {
  const projectName = useProjectStore(s => s.projectName)
  const dirty = useProjectStore(s => s.dirty)
  const label = hintText || `${dirty ? '*' : ''}${projectName} — Hardwave DAW`
  return (
    <div style={{
      fontSize: 11,
      color: hw.textFaint,
      overflow: 'hidden',
      textOverflow: 'ellipsis',
      whiteSpace: 'nowrap',
      maxWidth: 400,
      marginRight: 4,
    }}>
      {label}
    </div>
  )
}

const dropdownStyle: React.CSSProperties = {
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
}

interface MenuRowProps {
  item: MenuItem
  isSubmenuOpen: boolean
  onEnterSubmenu: () => void
  onLeaveSubmenu: () => void
  onClickItem: () => void
  onClickSubmenuItem: () => void
}

function MenuRow({ item, isSubmenuOpen, onEnterSubmenu, onLeaveSubmenu, onClickItem, onClickSubmenuItem }: MenuRowProps) {
  return (
    <div
      style={{ position: 'relative' }}
      onMouseEnter={() => { if (item.submenu) onEnterSubmenu(); else onLeaveSubmenu() }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          padding: '5px 12px',
          fontSize: 12,
          color: item.disabled ? hw.textFaint : hw.textSecondary,
          cursor: item.disabled ? 'default' : 'pointer',
          transition: 'background 0.1s, color 0.1s',
          background: isSubmenuOpen ? hw.accentDim : 'transparent',
        }}
        onMouseEnter={(e) => {
          if (!item.disabled) {
            e.currentTarget.style.background = hw.accentDim
            e.currentTarget.style.color = hw.textPrimary
          }
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.background = isSubmenuOpen ? hw.accentDim : 'transparent'
          e.currentTarget.style.color = item.disabled ? hw.textFaint : hw.textSecondary
        }}
        onClick={onClickItem}
      >
        <span>{item.label}</span>
        {item.shortcut && (
          <span style={{ fontSize: 10, color: hw.textFaint, marginLeft: 24, fontFamily: "'Consolas', monospace" }}>
            {item.shortcut}
          </span>
        )}
        {item.submenu && <span style={{ fontSize: 10, color: hw.textFaint, marginLeft: 24 }}>▶</span>}
      </div>

      {item.submenu && isSubmenuOpen && (
        <div style={{ ...dropdownStyle, top: 0, left: '100%', marginTop: 0, marginLeft: 2 }}>
          {item.submenu.map((sub, si) =>
            sub.separator ? (
              <div key={si} style={{ height: 1, margin: '4px 8px', background: hw.border }} />
            ) : (
              <div
                key={si}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  padding: '5px 12px',
                  fontSize: 12,
                  color: sub.disabled ? hw.textFaint : hw.textSecondary,
                  cursor: sub.disabled ? 'default' : 'pointer',
                }}
                onMouseEnter={(e) => {
                  if (!sub.disabled) {
                    e.currentTarget.style.background = hw.accentDim
                    e.currentTarget.style.color = hw.textPrimary
                  }
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.background = 'transparent'
                  e.currentTarget.style.color = sub.disabled ? hw.textFaint : hw.textSecondary
                }}
                onClick={() => {
                  if (sub.disabled || !sub.action) return
                  sub.action()
                  onClickSubmenuItem()
                }}
              >
                <span>{sub.label}</span>
              </div>
            )
          )}
        </div>
      )}
    </div>
  )
}

function shortenPath(p: string): string {
  const parts = p.replace(/\\/g, '/').split('/')
  const name = parts[parts.length - 1] || p
  return name.length > 40 ? name.slice(0, 37) + '...' : name
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
