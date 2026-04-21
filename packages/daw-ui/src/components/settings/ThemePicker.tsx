import { useRef, useState } from 'react'
import { hw } from '../../theme'
import {
  THEME_PRESETS,
  useThemeStore,
  isValidPalette,
  derivePaletteFromAccent,
  type ThemePalette,
} from '../../stores/themeStore'

interface ThemePickerProps {
  onClose: () => void
}

export function ThemePicker({ onClose }: ThemePickerProps) {
  const activeId = useThemeStore(s => s.activeId)
  const setTheme = useThemeStore(s => s.setTheme)
  const customs = useThemeStore(s => s.customs)
  const addCustom = useThemeStore(s => s.addCustom)
  const removeCustom = useThemeStore(s => s.removeCustom)
  const customBg = useThemeStore(s => s.customBg)
  const setCustomBg = useThemeStore(s => s.setCustomBg)
  const [bgDraft, setBgDraft] = useState(customBg)
  const [previewId, setPreviewId] = useState<string>(activeId)
  const [editorOpen, setEditorOpen] = useState(false)
  const [importError, setImportError] = useState<string | null>(null)
  const importRef = useRef<HTMLInputElement>(null)
  const allPalettes = [...THEME_PRESETS, ...customs]
  const preview = allPalettes.find(p => p.id === previewId) ?? THEME_PRESETS[0]
  const hasChanges = previewId !== activeId

  const applyAndReload = () => {
    setTheme(previewId)
    window.location.reload()
  }

  const exportCurrent = () => {
    const json = JSON.stringify(preview, null, 2)
    const blob = new Blob([json], { type: 'application/json' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `${preview.id}.hwtheme.json`
    document.body.appendChild(a)
    a.click()
    document.body.removeChild(a)
    URL.revokeObjectURL(url)
  }

  const onImportFile = async (e: React.ChangeEvent<HTMLInputElement>) => {
    setImportError(null)
    const file = e.target.files?.[0]
    e.target.value = ''
    if (!file) return
    try {
      const text = await file.text()
      const parsed = JSON.parse(text)
      if (!isValidPalette(parsed)) {
        setImportError('File is not a valid Hardwave theme.')
        return
      }
      const base = parsed.id.replace(/\s+/g, '-').toLowerCase()
      let id = base
      let i = 2
      while (allPalettes.some(p => p.id === id)) {
        id = `${base}-${i++}`
      }
      const palette: ThemePalette = { ...parsed, id, custom: true }
      if (!addCustom(palette)) {
        setImportError('Could not import (id clashes with a built-in).')
        return
      }
      setPreviewId(palette.id)
    } catch (err: any) {
      setImportError(`Could not read file: ${err?.message ?? err}`)
    }
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
            {allPalettes.map(p => (
              <PresetCard
                key={p.id}
                palette={p}
                selected={previewId === p.id}
                active={activeId === p.id}
                onPick={() => setPreviewId(p.id)}
                onDelete={p.custom ? () => {
                  removeCustom(p.id)
                  if (previewId === p.id) setPreviewId('hardwaveRed')
                } : undefined}
              />
            ))}
          </div>

          {/* Import / Export / Create */}
          <div style={{ display: 'flex', gap: 6, marginTop: 10, flexWrap: 'wrap' }}>
            <input
              ref={importRef}
              type="file"
              accept=".json,application/json"
              style={{ display: 'none' }}
              onChange={onImportFile}
            />
            <SmallBtn label="Create custom..." onClick={() => setEditorOpen(true)} />
            <SmallBtn label="Import theme..." onClick={() => importRef.current?.click()} />
            <SmallBtn label="Export current" onClick={exportCurrent} />
          </div>

          {importError && (
            <div style={{ marginTop: 8, fontSize: 11, color: hw.red, padding: '6px 10px', background: hw.redDim, borderRadius: hw.radius.sm }}>
              {importError}
            </div>
          )}

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

          {/* Custom background */}
          <div style={{
            marginTop: 14, padding: 12,
            background: hw.bgPanel, borderRadius: hw.radius.md,
            border: `1px solid ${hw.borderDark}`,
          }}>
            <div style={{ fontSize: 10, color: hw.textFaint, textTransform: 'uppercase', letterSpacing: 0.8, marginBottom: 8 }}>
              Custom background
            </div>
            <div style={{ fontSize: 11, color: hw.textMuted, marginBottom: 8 }}>
              Set any CSS background — a solid color, linear/radial gradient or <code style={{ color: hw.textSecondary }}>url(...)</code> image. Leave empty to restore the theme default.
            </div>
            <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
              <input
                type="text"
                value={bgDraft}
                onChange={e => setBgDraft(e.target.value)}
                placeholder="linear-gradient(135deg, #0a0a0e, #1a0505)"
                style={{
                  flex: 1, minWidth: 240,
                  fontSize: 11, padding: '6px 8px',
                  background: 'rgba(255,255,255,0.04)', color: hw.textPrimary,
                  border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
                  fontFamily: 'ui-monospace, Menlo, monospace',
                }}
              />
              <SmallBtn label="Apply" onClick={() => setCustomBg(bgDraft)} />
              <SmallBtn label="Clear" onClick={() => { setBgDraft(''); setCustomBg('') }} />
            </div>
            <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap', marginTop: 8 }}>
              <SmallBtn label="Crimson fade" onClick={() => {
                const v = 'radial-gradient(circle at 30% 20%, rgba(220,38,38,0.18), #06060a 55%)'
                setBgDraft(v); setCustomBg(v)
              }} />
              <SmallBtn label="Midnight diagonal" onClick={() => {
                const v = 'linear-gradient(135deg, #06060a 0%, #0b1226 60%, #06060a 100%)'
                setBgDraft(v); setCustomBg(v)
              }} />
              <SmallBtn label="Neon city" onClick={() => {
                const v = 'linear-gradient(180deg, #05060a 0%, #1a0530 50%, #030014 100%)'
                setBgDraft(v); setCustomBg(v)
              }} />
              {customBg && (
                <div style={{ alignSelf: 'center', fontSize: 10, color: hw.textFaint }}>
                  Active: custom background
                </div>
              )}
            </div>
          </div>
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
      {editorOpen && (
        <CustomThemeEditor
          existingIds={allPalettes.map(p => p.id)}
          onSave={(palette) => {
            if (addCustom(palette)) setPreviewId(palette.id)
          }}
          onClose={() => setEditorOpen(false)}
        />
      )}
    </div>
  )
}

function PresetCard({
  palette, selected, active, onPick, onDelete,
}: {
  palette: ThemePalette
  selected: boolean
  active: boolean
  onPick: () => void
  onDelete?: () => void
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
        position: 'relative',
      }}
    >
      {onDelete && (
        <button
          onClick={(e) => { e.stopPropagation(); onDelete() }}
          title="Delete custom theme"
          style={{
            position: 'absolute', top: 6, right: 6,
            width: 18, height: 18, borderRadius: 4,
            background: 'rgba(255,255,255,0.06)',
            border: `1px solid ${hw.border}`,
            color: hw.textFaint, cursor: 'pointer',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            padding: 0, fontSize: 10,
          }}
        >×</button>
      )}
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
            {palette.custom && (
              <span style={{
                fontSize: 8, fontWeight: 700, letterSpacing: 0.6,
                padding: '1px 5px', borderRadius: 3,
                color: hw.textMuted,
                background: 'rgba(255,255,255,0.06)',
                border: `1px solid ${hw.border}`,
              }}>CUSTOM</span>
            )}
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

function SmallBtn({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      style={{
        padding: '5px 10px', fontSize: 10, fontWeight: 600,
        color: hw.textSecondary,
        background: hw.bgElevated,
        border: `1px solid ${hw.border}`,
        borderRadius: hw.radius.sm, cursor: 'pointer',
        fontFamily: 'inherit', letterSpacing: 0.3,
      }}
    >
      {label}
    </button>
  )
}

function CustomThemeEditor({
  existingIds,
  onSave,
  onClose,
}: {
  existingIds: string[]
  onSave: (palette: ThemePalette) => void
  onClose: () => void
}) {
  const [name, setName] = useState('My Theme')
  const [description, setDescription] = useState('Custom palette')
  const [accent, setAccent] = useState('#DC2626')
  const [secondary, setSecondary] = useState('#B91C1C')
  const [error, setError] = useState<string | null>(null)

  const save = () => {
    const trimmed = name.trim()
    if (!trimmed) { setError('Name required.'); return }
    const base = `custom-${trimmed.replace(/\s+/g, '-').toLowerCase()}`.slice(0, 40)
    let id = base
    let i = 2
    while (existingIds.includes(id)) {
      id = `${base}-${i++}`
    }
    const palette = derivePaletteFromAccent(id, trimmed, description.trim() || 'Custom palette', accent, secondary)
    onSave(palette)
    onClose()
  }

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed', inset: 0, zIndex: 100,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        background: 'rgba(0,0,0,0.6)', backdropFilter: 'blur(4px)',
      }}
    >
      <div
        onClick={e => e.stopPropagation()}
        style={{
          width: 360, background: 'rgba(12,12,16,0.98)',
          border: `1px solid ${hw.borderLight}`,
          borderRadius: hw.radius.lg,
          boxShadow: '0 20px 60px rgba(0,0,0,0.7)', overflow: 'hidden',
        }}
      >
        <div style={{
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          padding: '12px 16px',
          background: 'rgba(255,255,255,0.03)',
          borderBottom: `1px solid ${hw.border}`,
        }}>
          <span style={{ fontSize: 13, fontWeight: 700, color: hw.accent, letterSpacing: 0.5 }}>
            NEW CUSTOM THEME
          </span>
        </div>
        <div style={{ padding: 14, display: 'flex', flexDirection: 'column', gap: 10 }}>
          <EditorField label="Name">
            <input
              value={name}
              onChange={e => setName(e.target.value)}
              style={editorInputStyle}
            />
          </EditorField>
          <EditorField label="Description">
            <input
              value={description}
              onChange={e => setDescription(e.target.value)}
              style={editorInputStyle}
            />
          </EditorField>
          <EditorField label="Accent">
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <input type="color" value={accent} onChange={e => setAccent(e.target.value)}
                style={{ width: 30, height: 24, border: 'none', background: 'transparent', cursor: 'pointer', padding: 0 }} />
              <input value={accent} onChange={e => setAccent(e.target.value)} style={editorInputStyle} />
            </div>
          </EditorField>
          <EditorField label="Secondary">
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <input type="color" value={secondary} onChange={e => setSecondary(e.target.value)}
                style={{ width: 30, height: 24, border: 'none', background: 'transparent', cursor: 'pointer', padding: 0 }} />
              <input value={secondary} onChange={e => setSecondary(e.target.value)} style={editorInputStyle} />
            </div>
          </EditorField>
          <div style={{
            marginTop: 4, padding: 10,
            background: hw.bgPanel, borderRadius: hw.radius.sm,
            border: `1px solid ${hw.borderDark}`,
            display: 'flex', alignItems: 'center', gap: 10,
          }}>
            <div style={{
              width: 32, height: 32, borderRadius: 6,
              background: `linear-gradient(135deg, ${secondary}, ${accent})`,
              boxShadow: `0 0 14px ${accent}55`,
            }} />
            <div style={{ fontSize: 11, color: hw.textMuted }}>
              Accent and secondary drive the full palette. Dim/glow/selection derive from accent alpha.
            </div>
          </div>
          {error && (
            <div style={{ fontSize: 11, color: hw.red, padding: '6px 10px', background: hw.redDim, borderRadius: hw.radius.sm }}>
              {error}
            </div>
          )}
        </div>
        <div style={{
          display: 'flex', justifyContent: 'flex-end', gap: 8,
          padding: '10px 16px',
          borderTop: `1px solid ${hw.border}`,
          background: 'rgba(255,255,255,0.02)',
        }}>
          <button onClick={onClose} style={smallFooterBtn(false)}>Cancel</button>
          <button onClick={save} style={smallFooterBtn(true)}>Save</button>
        </div>
      </div>
    </div>
  )
}

function EditorField({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
      <span style={{ fontSize: 10, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>{label}</span>
      {children}
    </div>
  )
}

const editorInputStyle: React.CSSProperties = {
  flex: 1, padding: '5px 8px',
  background: hw.bgInput, color: hw.textPrimary,
  border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
  fontSize: 12, fontFamily: 'inherit', outline: 'none',
}

function smallFooterBtn(primary: boolean): React.CSSProperties {
  return {
    padding: '6px 14px', fontSize: 11, fontWeight: 700,
    color: primary ? '#fff' : hw.textPrimary,
    background: primary ? hw.accent : hw.bgElevated,
    border: `1px solid ${primary ? hw.accent : hw.border}`,
    borderRadius: hw.radius.sm, cursor: 'pointer',
    fontFamily: 'inherit',
  }
}
