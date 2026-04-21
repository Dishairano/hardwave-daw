import { useEffect, useMemo, useRef, useState } from 'react'
import { hw } from '../theme'
import {
  ACTIONS, PRESETS,
  useShortcutsStore,
  bindingLabel,
  isReservedBinding,
  type ActionId, type Binding,
} from '../stores/shortcutsStore'

interface Category {
  name: string
  actions: typeof ACTIONS
}

function groupByCategory(): Category[] {
  const map = new Map<string, typeof ACTIONS>()
  for (const a of ACTIONS) {
    const list = map.get(a.category) ?? []
    list.push(a)
    map.set(a.category, list)
  }
  return Array.from(map.entries()).map(([name, actions]) => ({ name, actions }))
}

const CATEGORIES = groupByCategory()

export function ShortcutsPanel({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [query, setQuery] = useState('')
  const bindings = useShortcutsStore(s => s.bindings)
  const capturingFor = useShortcutsStore(s => s.capturingFor)
  const startCapture = useShortcutsStore(s => s.startCapture)
  const setBinding = useShortcutsStore(s => s.setBinding)
  const resetAll = useShortcutsStore(s => s.resetAll)
  const loadPreset = useShortcutsStore(s => s.loadPreset)
  const exportJson = useShortcutsStore(s => s.exportJson)
  const importJson = useShortcutsStore(s => s.importJson)
  const findConflict = useShortcutsStore(s => s.findConflict)

  const [banner, setBanner] = useState<{ kind: 'info' | 'error' | 'warn'; text: string } | null>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)

  // Escape to close, even while capturing (capture aborts first)
  useEffect(() => {
    if (!open) return
    const handler = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.preventDefault()
      if (useShortcutsStore.getState().capturingFor) {
        startCapture(null)
      } else {
        onClose()
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [open, onClose, startCapture])

  // Capture handler — when a row is "capturing", swallow the next key event
  useEffect(() => {
    if (!open || !capturingFor) return
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') return // Escape already handled above (cancels capture)
      // Ignore bare modifier presses — user is still composing the combo
      if (['Control', 'Shift', 'Alt', 'Meta', 'OS'].includes(e.key)) return
      e.preventDefault()
      e.stopPropagation()
      const b: Binding = {
        code: e.code,
        ctrl: e.ctrlKey || e.metaKey,
        shift: e.shiftKey,
        alt: e.altKey,
      }
      if (isReservedBinding(b)) {
        setBanner({ kind: 'error', text: 'That combination cannot be used as a shortcut.' })
        startCapture(null)
        return
      }
      const conflict = findConflict(b, capturingFor)
      if (conflict) {
        const otherLabel = ACTIONS.find(a => a.id === conflict)?.label ?? conflict
        setBanner({
          kind: 'warn',
          text: `${bindingLabel(b)} was bound to “${otherLabel}” — that shortcut is now unassigned (reset to default).`,
        })
        // Reset the conflicting action to its default; then assign new binding
        setBinding(conflict, null)
        setBinding(capturingFor, b)
      } else {
        setBanner(null)
        setBinding(capturingFor, b)
      }
    }
    window.addEventListener('keydown', handler, true)
    return () => window.removeEventListener('keydown', handler, true)
  }, [open, capturingFor, setBinding, startCapture, findConflict])

  const filteredCategories = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return CATEGORIES
    return CATEGORIES
      .map(cat => ({
        ...cat,
        actions: cat.actions.filter(a =>
          a.label.toLowerCase().includes(q)
          || bindingLabel(bindings[a.id]).toLowerCase().includes(q),
        ),
      }))
      .filter(cat => cat.actions.length > 0)
  }, [query, bindings])

  if (!open) return null

  const downloadJson = () => {
    const blob = new Blob([exportJson()], { type: 'application/json' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = 'hardwave-shortcuts.json'
    document.body.appendChild(a)
    a.click()
    document.body.removeChild(a)
    URL.revokeObjectURL(url)
    setBanner({ kind: 'info', text: 'Shortcut map exported to hardwave-shortcuts.json.' })
  }

  const onImportFile = (file: File) => {
    file.text().then(text => {
      const result = importJson(text)
      if (result.ok) {
        setBanner({ kind: 'info', text: 'Shortcut map imported successfully.' })
      } else {
        setBanner({ kind: 'error', text: `Import failed: ${result.error}` })
      }
    })
  }

  const bannerColor =
    banner?.kind === 'error' ? hw.red :
    banner?.kind === 'warn' ? hw.yellow :
    hw.accent

  return (
    <div
      onMouseDown={onClose}
      style={{
        position: 'fixed', inset: 0, zIndex: 15000,
        background: 'rgba(0,0,0,0.55)', backdropFilter: hw.blur.sm,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
    >
      <div
        onMouseDown={e => e.stopPropagation()}
        style={{
          width: 'min(760px, 92vw)', maxHeight: '82vh',
          display: 'flex', flexDirection: 'column',
          background: 'rgba(12,12,18,0.98)', border: `1px solid ${hw.borderLight}`,
          borderRadius: hw.radius.lg, boxShadow: '0 16px 48px rgba(0,0,0,0.6)',
        }}
      >
        {/* Header */}
        <div style={{
          padding: '12px 16px', borderBottom: `1px solid ${hw.border}`,
          display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap',
        }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: hw.textPrimary, marginRight: 'auto' }}>
            Keyboard shortcuts
          </div>
          <input
            autoFocus
            placeholder="Search…"
            value={query}
            onChange={e => setQuery(e.target.value)}
            style={{
              flex: '0 0 180px', fontSize: 11, color: hw.textPrimary,
              background: 'rgba(255,255,255,0.04)',
              border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
              padding: '4px 8px', outline: 'none',
            }}
          />
          <select
            defaultValue=""
            onChange={e => {
              const id = e.target.value
              if (!id) return
              loadPreset(id)
              setBanner({ kind: 'info', text: `Loaded preset: ${PRESETS.find(p => p.id === id)?.name}` })
              e.target.value = ''
            }}
            style={{
              fontSize: 11, color: hw.textPrimary,
              background: 'rgba(255,255,255,0.04)',
              border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
              padding: '3px 6px', outline: 'none',
            }}
          >
            <option value="" disabled>Load preset…</option>
            {PRESETS.map(p => (
              <option key={p.id} value={p.id}>{p.name}</option>
            ))}
          </select>
          <HeaderBtn onClick={downloadJson}>Export</HeaderBtn>
          <HeaderBtn onClick={() => fileInputRef.current?.click()}>Import</HeaderBtn>
          <input
            ref={fileInputRef}
            type="file"
            accept=".json,application/json"
            style={{ display: 'none' }}
            onChange={e => {
              const f = e.target.files?.[0]
              if (f) onImportFile(f)
              e.target.value = ''
            }}
          />
          <HeaderBtn onClick={() => {
            resetAll()
            setBanner({ kind: 'info', text: 'All shortcuts reset to defaults.' })
          }}>Reset</HeaderBtn>
          <HeaderBtn onClick={() => printShortcuts(filteredCategories, bindings)}>Print</HeaderBtn>
          <HeaderBtn onClick={onClose}>Close</HeaderBtn>
        </div>

        {banner && (
          <div style={{
            padding: '8px 16px', fontSize: 11,
            background: `${bannerColor}15`,
            borderBottom: `1px solid ${bannerColor}40`,
            color: bannerColor,
            display: 'flex', alignItems: 'center', gap: 8,
          }}>
            <span style={{ flex: 1 }}>{banner.text}</span>
            <span
              onClick={() => setBanner(null)}
              style={{ cursor: 'pointer', opacity: 0.7, fontSize: 12 }}
            >✕</span>
          </div>
        )}

        {capturingFor && (
          <div style={{
            padding: '8px 16px', fontSize: 11,
            background: hw.accentDim,
            borderBottom: `1px solid ${hw.accent}40`,
            color: hw.accentLight,
          }}>
            Press a key combo for <strong>{ACTIONS.find(a => a.id === capturingFor)?.label}</strong> — Esc to cancel.
          </div>
        )}

        <div style={{ overflowY: 'auto', padding: 12 }}>
          {filteredCategories.length === 0 && (
            <div style={{ padding: 24, textAlign: 'center', color: hw.textFaint, fontSize: 11 }}>
              No shortcuts match "{query}"
            </div>
          )}
          {filteredCategories.map(cat => (
            <div key={cat.name} style={{ marginBottom: 14 }}>
              <div style={{
                fontSize: 9, color: hw.textFaint, letterSpacing: 0.6,
                textTransform: 'uppercase', marginBottom: 4, padding: '0 4px',
              }}>
                {cat.name}
              </div>
              {cat.actions.map(a => {
                const isCapturing = capturingFor === a.id
                return (
                  <div
                    key={a.id}
                    style={{
                      display: 'flex', alignItems: 'center', gap: 12,
                      padding: '4px 8px', borderRadius: hw.radius.sm,
                      background: isCapturing ? hw.accentDim : 'rgba(255,255,255,0.015)',
                      border: `1px solid ${isCapturing ? hw.accent : 'transparent'}`,
                      marginBottom: 2,
                    }}
                  >
                    <span
                      onClick={() => startCapture(isCapturing ? null : a.id)}
                      style={{
                        fontSize: 10, fontFamily: 'ui-monospace, Menlo, monospace',
                        color: isCapturing ? hw.accentLight : hw.accent,
                        minWidth: 200, padding: '2px 6px',
                        background: isCapturing ? 'rgba(220,38,38,0.25)' : 'rgba(255,255,255,0.04)',
                        border: `1px solid ${isCapturing ? hw.accent : hw.border}`,
                        borderRadius: hw.radius.sm, cursor: 'pointer',
                        textAlign: 'center',
                      }}
                      title="Click to rebind"
                    >
                      {isCapturing ? 'Press a key…' : bindingLabel(bindings[a.id])}
                    </span>
                    <span style={{ fontSize: 11, color: hw.textSecondary, flex: 1 }}>
                      {a.label}
                    </span>
                    <span
                      onClick={() => {
                        setBinding(a.id, null)
                        setBanner({ kind: 'info', text: `“${a.label}” restored to default.` })
                      }}
                      style={{
                        fontSize: 9, color: hw.textFaint,
                        padding: '2px 6px', borderRadius: hw.radius.sm,
                        cursor: 'pointer', opacity: 0.6,
                      }}
                      title="Reset this shortcut to default"
                    >↺</span>
                  </div>
                )
              })}
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}

function HeaderBtn({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      style={{
        padding: '3px 10px', fontSize: 10, fontWeight: 600,
        color: hw.textMuted, background: 'rgba(255,255,255,0.04)',
        border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm, cursor: 'pointer',
      }}
    >
      {children}
    </button>
  )
}

function printShortcuts(cats: Category[], bindings: Record<ActionId, Binding>) {
  const win = window.open('', '_blank', 'width=720,height=900')
  if (!win) return
  const rows = cats
    .map(c => {
      const items = c.actions
        .map(a => `<tr><td class="k">${escapeHtml(bindingLabel(bindings[a.id]))}</td><td>${escapeHtml(a.label)}</td></tr>`)
        .join('')
      return `<h2>${escapeHtml(c.name)}</h2><table>${items}</table>`
    })
    .join('')
  win.document.write(`<!doctype html><html><head><meta charset="utf-8"/><title>Hardwave DAW — Keyboard Shortcuts</title>
<style>
body{font:12px/1.45 -apple-system,Segoe UI,Roboto,sans-serif;color:#111;padding:24px;max-width:720px;margin:0 auto}
h1{font-size:18px;margin:0 0 4px}
.sub{color:#666;font-size:11px;margin-bottom:18px}
h2{font-size:13px;margin:18px 0 6px;padding-bottom:2px;border-bottom:1px solid #ccc;text-transform:uppercase;letter-spacing:0.05em;color:#333}
table{width:100%;border-collapse:collapse;margin-bottom:10px}
td{padding:3px 6px;border-bottom:1px solid #eee;vertical-align:top}
td.k{font-family:ui-monospace,Menlo,monospace;font-size:11px;color:#b91c1c;width:220px}
@media print{body{padding:12px}h2{page-break-after:avoid}table{page-break-inside:auto}tr{page-break-inside:avoid}}
</style></head><body>
<h1>Hardwave DAW — Keyboard shortcuts</h1>
<div class="sub">Generated from the in-app shortcuts panel.</div>
${rows}
</body></html>`)
  win.document.close()
  win.focus()
  setTimeout(() => {
    try { win.print() } catch { /* ignore popup blocker quirks */ }
  }, 150)
}

function escapeHtml(s: string) {
  return s.replace(/[&<>"']/g, ch => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
  }[ch] as string))
}
