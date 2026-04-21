import { useState, useMemo, useEffect } from 'react'
import { hw } from '../theme'

interface Shortcut {
  keys: string
  label: string
}

interface Category {
  name: string
  items: Shortcut[]
}

const CATEGORIES: Category[] = [
  {
    name: 'Transport',
    items: [
      { keys: 'Space', label: 'Play / pause' },
      { keys: 'Home', label: 'Return to start' },
      { keys: 'End', label: 'Jump to project end' },
      { keys: 'L', label: 'Toggle loop region' },
      { keys: 'Alt + →', label: 'Jump to next marker' },
      { keys: 'Alt + ←', label: 'Jump to previous marker' },
      { keys: 'Shift + M', label: 'Drop marker at playhead' },
    ],
  },
  {
    name: 'Panels',
    items: [
      { keys: 'F5', label: 'Toggle Playlist' },
      { keys: 'F6', label: 'Toggle Channel Rack' },
      { keys: 'F7', label: 'Toggle Piano Roll' },
      { keys: 'F8', label: 'Toggle Browser' },
      { keys: 'F9', label: 'Toggle Mixer' },
      { keys: '?', label: 'Toggle this help panel' },
    ],
  },
  {
    name: 'Project',
    items: [
      { keys: 'Ctrl + N', label: 'New project' },
      { keys: 'Ctrl + O', label: 'Open project' },
      { keys: 'Ctrl + S', label: 'Save project' },
      { keys: 'Ctrl + Shift + S', label: 'Save project as…' },
      { keys: 'Ctrl + Z', label: 'Undo' },
      { keys: 'Ctrl + Shift + Z / Ctrl + Y', label: 'Redo' },
    ],
  },
  {
    name: 'Arrangement — clips',
    items: [
      { keys: 'Ctrl + C / X / V', label: 'Copy / cut / paste clips' },
      { keys: 'Ctrl + D', label: 'Duplicate selected clip' },
      { keys: 'Delete / Backspace', label: 'Delete selected clips' },
      { keys: 'S', label: 'Split clip at playhead / edit cursor' },
      { keys: 'Click ruler', label: 'Seek playhead' },
      { keys: 'Right-click ruler', label: 'Marker menu / add marker' },
      { keys: 'Ctrl + Scroll', label: 'Horizontal zoom' },
      { keys: 'Ctrl + Shift + Scroll', label: 'Vertical zoom (track height)' },
    ],
  },
  {
    name: 'Piano Roll — notes',
    items: [
      { keys: 'Ctrl + A', label: 'Select all notes' },
      { keys: 'Shift + Click', label: 'Toggle a note in the selection' },
      { keys: 'Delete', label: 'Delete selected notes' },
      { keys: '↑ / ↓', label: 'Transpose ±1 semitone' },
      { keys: 'Ctrl + ↑ / ↓', label: 'Transpose ±1 octave' },
      { keys: '← / →', label: 'Nudge by grid step' },
      { keys: 'Ctrl + D', label: 'Duplicate selection' },
      { keys: 'Ctrl + V', label: 'Paste at playhead' },
      { keys: 'Ctrl + Shift + V', label: 'Paste at original position' },
      { keys: 'Ctrl + Q', label: 'Quantize selection' },
      { keys: 'Ctrl + Alt + A', label: 'Arpeggiate selection' },
    ],
  },
  {
    name: 'Piano Roll — tools',
    items: [
      { keys: 'D', label: 'Draw tool' },
      { keys: 'S', label: 'Select tool' },
      { keys: 'E', label: 'Eraser tool' },
      { keys: 'C', label: 'Chord stamp' },
      { keys: 'Alt + Click', label: 'Switch to ghost-note source pattern' },
      { keys: 'Middle-mouse drag', label: 'Pan canvas' },
    ],
  },
  {
    name: 'Mixer & tracks',
    items: [
      { keys: 'Double-click track name', label: 'Rename track' },
      { keys: 'Right-click track / strip', label: 'Open context menu (color, mute, delete…)' },
      { keys: 'Click strip color bar', label: 'Change track color' },
    ],
  },
]

export function ShortcutsPanel({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [query, setQuery] = useState('')

  useEffect(() => {
    if (!open) return
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { e.preventDefault(); onClose() }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [open, onClose])

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return CATEGORIES
    return CATEGORIES
      .map(cat => ({
        ...cat,
        items: cat.items.filter(it =>
          it.keys.toLowerCase().includes(q) || it.label.toLowerCase().includes(q)
        ),
      }))
      .filter(cat => cat.items.length > 0)
  }, [query])

  if (!open) return null

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
          width: 'min(720px, 92vw)', maxHeight: '80vh',
          display: 'flex', flexDirection: 'column',
          background: 'rgba(12,12,18,0.98)', border: `1px solid ${hw.borderLight}`,
          borderRadius: hw.radius.lg, boxShadow: '0 16px 48px rgba(0,0,0,0.6)',
        }}
      >
        <div style={{
          padding: '12px 16px', borderBottom: `1px solid ${hw.border}`,
          display: 'flex', alignItems: 'center', gap: 12,
        }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: hw.textPrimary }}>
            Keyboard shortcuts
          </div>
          <input
            autoFocus
            placeholder="Search…"
            value={query}
            onChange={e => setQuery(e.target.value)}
            style={{
              flex: 1, fontSize: 11, color: hw.textPrimary,
              background: 'rgba(255,255,255,0.04)',
              border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
              padding: '4px 8px', outline: 'none',
            }}
          />
          <button
            onClick={() => printShortcuts(filtered)}
            title="Open a printable cheat sheet in a new window"
            style={{
              padding: '3px 10px', fontSize: 10, fontWeight: 600,
              color: hw.textMuted, background: 'rgba(255,255,255,0.04)',
              border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm, cursor: 'pointer',
            }}
          >
            Print
          </button>
          <button
            onClick={onClose}
            style={{
              padding: '3px 10px', fontSize: 10, fontWeight: 600,
              color: hw.textMuted, background: 'rgba(255,255,255,0.04)',
              border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm, cursor: 'pointer',
            }}
          >
            Close
          </button>
        </div>
        <div style={{ overflowY: 'auto', padding: 12 }}>
          {filtered.length === 0 && (
            <div style={{ padding: 24, textAlign: 'center', color: hw.textFaint, fontSize: 11 }}>
              No shortcuts match "{query}"
            </div>
          )}
          {filtered.map(cat => (
            <div key={cat.name} style={{ marginBottom: 14 }}>
              <div style={{
                fontSize: 9, color: hw.textFaint, letterSpacing: 0.6,
                textTransform: 'uppercase', marginBottom: 4, padding: '0 4px',
              }}>
                {cat.name}
              </div>
              {cat.items.map(it => (
                <div key={it.keys + it.label} style={{
                  display: 'flex', alignItems: 'center', gap: 12,
                  padding: '4px 8px', borderRadius: hw.radius.sm,
                  background: 'rgba(255,255,255,0.015)', marginBottom: 2,
                }}>
                  <span style={{
                    fontSize: 10, fontFamily: 'ui-monospace, Menlo, monospace',
                    color: hw.accent, minWidth: 180,
                  }}>
                    {it.keys}
                  </span>
                  <span style={{ fontSize: 11, color: hw.textSecondary, flex: 1 }}>
                    {it.label}
                  </span>
                </div>
              ))}
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}

function printShortcuts(cats: Category[]) {
  const win = window.open('', '_blank', 'width=720,height=900')
  if (!win) return
  const rows = cats
    .map(c => {
      const items = c.items
        .map(it => `<tr><td class="k">${escapeHtml(it.keys)}</td><td>${escapeHtml(it.label)}</td></tr>`)
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
