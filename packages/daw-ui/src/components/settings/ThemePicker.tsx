import { useState } from 'react'
import { hw } from '../../theme'
import { THEME_PRESETS, useThemeStore, type ThemePalette } from '../../stores/themeStore'

interface ThemePickerProps {
  onClose: () => void
}

export function ThemePicker({ onClose }: ThemePickerProps) {
  const activeId = useThemeStore(s => s.activeId)
  const setTheme = useThemeStore(s => s.setTheme)
  const [previewId, setPreviewId] = useState<string>(activeId)
  const preview = THEME_PRESETS.find(p => p.id === previewId) ?? THEME_PRESETS[0]
  const hasChanges = previewId !== activeId

  const applyAndReload = () => {
    setTheme(previewId)
    window.location.reload()
  }

  return (
    <div
      style={{
        position: 'fixed', inset: 0, zIndex: 90,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        background: 'rgba(0,0,0,0.6)', backdropFilter: 'blur(4px)',
      }}
      onClick={onClose}
    >
      <div
        onClick={e => e.stopPropagation()}
        style={{
          width: 560, background: 'rgba(12,12,16,0.98)',
          border: `1px solid ${hw.borderLight}`,
          borderRadius: hw.radius.lg,
          boxShadow: '0 20px 60px rgba(0,0,0,0.7)',
          overflow: 'hidden',
        }}
      >
        {/* Header */}
        <div style={{
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          padding: '12px 16px',
          background: 'rgba(255,255,255,0.03)',
          borderBottom: `1px solid ${hw.border}`,
        }}>
          <span style={{ fontSize: 13, fontWeight: 700, color: hw.accent, letterSpacing: 0.5 }}>
            THEME
          </span>
          <div
            onClick={onClose}
            style={{
              width: 20, height: 20, display: 'flex', alignItems: 'center', justifyContent: 'center',
              borderRadius: 4, color: hw.textFaint, cursor: 'pointer',
            }}
            onMouseEnter={e => { e.currentTarget.style.background = 'rgba(255,255,255,0.08)'; e.currentTarget.style.color = hw.textPrimary }}
            onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = hw.textFaint }}
          >
            <svg width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
              <line x1="2" y1="2" x2="10" y2="10" /><line x1="10" y1="2" x2="2" y2="10" />
            </svg>
          </div>
        </div>

        {/* Body */}
        <div style={{ padding: 16 }}>
          <div style={{ fontSize: 11, color: hw.textMuted, marginBottom: 12 }}>
            Pick an accent palette. Background and text stay neutral — only accents, selection, and glow shift.
          </div>

          <div style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(2, 1fr)',
            gap: 8,
          }}>
            {THEME_PRESETS.map(p => (
              <PresetCard
                key={p.id}
                palette={p}
                selected={previewId === p.id}
                active={activeId === p.id}
                onPick={() => setPreviewId(p.id)}
              />
            ))}
          </div>

          {/* Preview strip */}
          <div style={{
            marginTop: 14, padding: 12,
            background: hw.bgPanel, borderRadius: hw.radius.md,
            border: `1px solid ${hw.borderDark}`,
          }}>
            <div style={{ fontSize: 10, color: hw.textFaint, textTransform: 'uppercase', letterSpacing: 0.8, marginBottom: 8 }}>
              Preview — {preview.name}
            </div>
            <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
              <Swatch color={preview.accent} label="accent" />
              <Swatch color={preview.accentLight} label="light" />
              <Swatch color={preview.secondary} label="secondary" />
              <div style={{ flex: 1 }} />
              <div style={{
                padding: '4px 10px',
                background: preview.accent, color: '#fff',
                fontSize: 10, fontWeight: 700, borderRadius: hw.radius.sm,
              }}>BUTTON</div>
              <div style={{
                padding: '4px 10px',
                background: preview.selectionDim,
                border: `1px solid ${preview.accent}`,
                color: preview.accentLight,
                fontSize: 10, fontWeight: 600, borderRadius: hw.radius.sm,
              }}>SELECTED</div>
            </div>
          </div>

          {hasChanges && (
            <div style={{
              marginTop: 10, padding: '8px 12px',
              background: 'rgba(245,158,11,0.08)',
              border: `1px solid rgba(245,158,11,0.25)`,
              borderRadius: hw.radius.sm,
              fontSize: 11, color: hw.yellow,
            }}>
              Applying a theme reloads the window so every panel picks up the new accents.
            </div>
          )}
        </div>

        {/* Footer */}
        <div style={{
          display: 'flex', justifyContent: 'flex-end', gap: 8,
          padding: '10px 16px',
          borderTop: `1px solid ${hw.border}`,
          background: 'rgba(255,255,255,0.02)',
        }}>
          <button
            onClick={onClose}
            style={{
              padding: '6px 14px', fontSize: 11, fontWeight: 600,
              color: hw.textPrimary,
              background: hw.bgElevated,
              border: `1px solid ${hw.border}`,
              borderRadius: hw.radius.sm, cursor: 'pointer',
            }}
          >
            Cancel
          </button>
          <button
            onClick={applyAndReload}
            disabled={!hasChanges}
            style={{
              padding: '6px 14px', fontSize: 11, fontWeight: 700,
              color: '#fff',
              background: hasChanges ? hw.accent : hw.bgElevated,
              border: `1px solid ${hasChanges ? hw.accent : hw.border}`,
              borderRadius: hw.radius.sm,
              cursor: hasChanges ? 'pointer' : 'default',
              opacity: hasChanges ? 1 : 0.5,
            }}
          >
            Apply & Reload
          </button>
        </div>
      </div>
    </div>
  )
}

function PresetCard({
  palette, selected, active, onPick,
}: {
  palette: ThemePalette
  selected: boolean
  active: boolean
  onPick: () => void
}) {
  return (
    <div
      onClick={onPick}
      style={{
        padding: 10,
        background: selected ? palette.selectionDim : hw.bgPanel,
        border: `1px solid ${selected ? palette.accent : hw.borderDark}`,
        borderRadius: hw.radius.md,
        cursor: 'pointer',
        transition: 'background 0.1s, border-color 0.1s',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
        <div style={{
          width: 22, height: 22, borderRadius: 6,
          background: `linear-gradient(135deg, ${palette.secondary}, ${palette.accent})`,
          boxShadow: `0 0 10px ${palette.accentGlow}`,
        }} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{
            display: 'flex', alignItems: 'center', gap: 6,
            fontSize: 12, fontWeight: 600, color: hw.textPrimary,
          }}>
            {palette.name}
            {active && (
              <span style={{
                fontSize: 8, fontWeight: 700, letterSpacing: 0.6,
                padding: '1px 5px', borderRadius: 3,
                color: palette.accent,
                background: palette.selectionDim,
                border: `1px solid ${palette.accent}`,
              }}>ACTIVE</span>
            )}
          </div>
          <div style={{
            fontSize: 10, color: hw.textFaint,
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>{palette.description}</div>
        </div>
      </div>
      <div style={{ display: 'flex', gap: 4 }}>
        <MiniSwatch color={palette.accent} />
        <MiniSwatch color={palette.accentLight} />
        <MiniSwatch color={palette.secondary} />
        <MiniSwatch color={palette.selection} />
      </div>
    </div>
  )
}

function Swatch({ color, label }: { color: string; label: string }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 2, alignItems: 'center' }}>
      <div style={{
        width: 22, height: 22, borderRadius: 4,
        background: color,
        border: `1px solid ${hw.border}`,
      }} />
      <div style={{ fontSize: 8, color: hw.textFaint, letterSpacing: 0.3 }}>{label}</div>
    </div>
  )
}

function MiniSwatch({ color }: { color: string }) {
  return (
    <div style={{
      flex: 1, height: 6, borderRadius: 2,
      background: color,
    }} />
  )
}
