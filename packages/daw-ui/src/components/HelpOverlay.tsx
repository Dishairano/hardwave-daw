import { useEffect, useMemo, useState } from 'react'
import { hw } from '../theme'
import { ACTIONS, bindingLabel, useShortcutsStore, type ActionId } from '../stores/shortcutsStore'

interface Topic {
  id: string
  title: string
  panel: string
  body: string
  shortcuts?: ActionId[]
  keywords?: string[]
}

const TOPICS: Topic[] = [
  {
    id: 'transport',
    title: 'Transport',
    panel: 'Title bar',
    body: 'Play, stop, record, tempo, time signature, and metronome live in the top bar. Click the play button or press Space to toggle playback. The tempo LCD is drag-to-scrub and double-click-to-type.',
    shortcuts: ['togglePlay', 'gotoStart', 'toggleLoop'],
    keywords: ['play', 'stop', 'bpm', 'tempo', 'metronome', 'record'],
  },
  {
    id: 'arrangement',
    title: 'Arrangement',
    panel: 'Main canvas',
    body: 'The horizontal timeline where clips are arranged per track. Drag audio from the Browser to drop a clip. Click a clip to select it, Ctrl+click to multi-select, drag-select with the left mouse button on empty space. Right-click any clip for its menu (Rename, Colour, Fades, Group, Duplicate, Delete). Overlapping clips can be auto-crossfaded from Edit > Auto-crossfade overlaps.',
    shortcuts: ['deleteSelection', 'duplicate', 'selectAll', 'undo', 'redo'],
    keywords: ['clip', 'track', 'timeline', 'crossfade', 'fade', 'group'],
  },
  {
    id: 'piano-roll',
    title: 'Piano Roll',
    panel: 'Notes editor',
    body: 'Double-click a MIDI clip in the arrangement to open the Piano Roll. Draw notes with the pencil tool, move them by dragging, resize by dragging their right edge. Velocity is edited in the lane below. Ghost notes from other patterns can be overlaid for reference. Quantize and groove templates live in the Tools menu.',
    shortcuts: ['selectAll', 'deleteSelection', 'undo'],
    keywords: ['midi', 'notes', 'velocity', 'quantize', 'pattern'],
  },
  {
    id: 'mixer',
    title: 'Mixer',
    panel: 'Channel strips',
    body: 'One channel strip per track plus the Master. Faders adjust output level, the pan knob pans the stereo field, and each strip has mute/solo buttons. Metering shows peak level; the Master strip also shows LUFS (from View > Loudness Meter). Signal flow is track → Master.',
    keywords: ['fader', 'pan', 'mute', 'solo', 'master', 'meter', 'lufs'],
  },
  {
    id: 'channel-rack',
    title: 'Channel Rack',
    panel: 'Step sequencer',
    body: 'A step-based sequencer for drum patterns. Each row is a channel, each column is a step (at the current pattern resolution). Click a step to toggle it on. The pattern is then triggered from clips in the Arrangement.',
    keywords: ['step', 'sequencer', 'drums', 'pattern', 'beat'],
  },
  {
    id: 'browser',
    title: 'Browser',
    panel: 'Left panel',
    body: 'Navigate your sample library, presets, and recent projects. Drag any audio file onto a track to import it as a clip. Use the search box to filter by filename. Add custom folders from Settings > Audio & Paths.',
    keywords: ['samples', 'library', 'files', 'import', 'drag'],
  },
  {
    id: 'automation',
    title: 'Automation',
    panel: 'Per-parameter lanes',
    body: 'Record-arm a parameter and play back to capture changes, or draw curve points directly. Right-click an automation point for options (curve type, value, delete). Automation clips appear as a distinct clip type in the arrangement.',
    keywords: ['automation', 'curve', 'lfo', 'modulation'],
  },
  {
    id: 'shortcuts',
    title: 'Keyboard shortcuts',
    panel: 'Global',
    body: 'Every action can be rebound from the Shortcuts panel (Help > Keyboard shortcuts). Export / import shortcut maps as JSON to move them between installations, or load one of the built-in presets (FL Studio, Ableton, Logic).',
    shortcuts: ['undo', 'redo', 'cut', 'copy', 'paste'],
    keywords: ['keymap', 'binding', 'hotkey', 'preset'],
  },
  {
    id: 'project',
    title: 'Projects & saves',
    panel: 'File menu',
    body: 'Create a new project from File > New, open an existing one with Ctrl+O, save with Ctrl+S. The most recent projects are listed under File > Open Recent. Auto-save and crash-recovery snapshots are taken in the background — on startup you will be offered the most recent snapshot if one exists.',
    shortcuts: ['newProject', 'openProject', 'save', 'saveAs'],
    keywords: ['save', 'open', 'new', 'project', 'file', 'crash', 'recovery'],
  },
  {
    id: 'loudness',
    title: 'Loudness Meter',
    panel: 'View > Loudness Meter',
    body: 'Measures the Master bus integrated LUFS, short-term LUFS, and true-peak. Use the Reset button (or start transport from zero) to clear history between takes. The target line at -14 LUFS matches streaming loudness norms.',
    keywords: ['lufs', 'meter', 'loudness', 'peak', 'master'],
  },
  {
    id: 'history',
    title: 'Undo history',
    panel: 'View > History',
    body: 'Every destructive edit (move, trim, delete, rename, automation) is pushed to the undo history. Undo / redo walk one step at a time; the History panel lets you jump to any earlier state in the current session.',
    shortcuts: ['undo', 'redo'],
    keywords: ['undo', 'redo', 'history', 'timeline'],
  },
]

function topicScore(topic: Topic, query: string): number {
  if (!query) return 1
  const q = query.toLowerCase()
  let score = 0
  if (topic.title.toLowerCase().includes(q)) score += 4
  if (topic.panel.toLowerCase().includes(q)) score += 2
  if (topic.body.toLowerCase().includes(q)) score += 1
  for (const kw of topic.keywords ?? []) {
    if (kw.toLowerCase().includes(q)) score += 2
  }
  return score
}

export function HelpOverlay({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [query, setQuery] = useState('')
  const [activeId, setActiveId] = useState<string>(TOPICS[0].id)
  const bindings = useShortcutsStore(s => s.bindings)

  useEffect(() => {
    if (!open) return
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        onClose()
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [open, onClose])

  useEffect(() => {
    if (open) setQuery('')
  }, [open])

  const ranked = useMemo(() => {
    const q = query.trim()
    if (!q) return TOPICS
    return TOPICS
      .map(t => ({ t, s: topicScore(t, q) }))
      .filter(x => x.s > 0)
      .sort((a, b) => b.s - a.s)
      .map(x => x.t)
  }, [query])

  useEffect(() => {
    if (ranked.length > 0 && !ranked.some(t => t.id === activeId)) {
      setActiveId(ranked[0].id)
    }
  }, [ranked, activeId])

  if (!open) return null

  const active = ranked.find(t => t.id === activeId) ?? ranked[0] ?? TOPICS[0]

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
          width: 'min(820px, 94vw)', height: 'min(560px, 82vh)',
          display: 'flex', flexDirection: 'column',
          background: 'rgba(12,12,18,0.98)', border: `1px solid ${hw.borderLight}`,
          borderRadius: hw.radius.lg, boxShadow: '0 16px 48px rgba(0,0,0,0.6)',
        }}
      >
        <div style={{
          padding: '12px 16px', borderBottom: `1px solid ${hw.border}`,
          display: 'flex', alignItems: 'center', gap: 12,
        }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: hw.textPrimary, marginRight: 'auto' }}>
            Help
            <span style={{ fontSize: 10, fontWeight: 400, color: hw.textFaint, marginLeft: 8 }}>
              F1 toggles this overlay
            </span>
          </div>
          <input
            autoFocus
            placeholder="Search topics…"
            value={query}
            onChange={e => setQuery(e.target.value)}
            style={{
              flex: '0 0 220px', fontSize: 11, color: hw.textPrimary,
              background: hw.bgInput,
              border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
              padding: '4px 8px', outline: 'none',
            }}
          />
          <button
            onClick={onClose}
            style={{
              padding: '3px 10px', fontSize: 10, fontWeight: 600,
              color: hw.textMuted, background: hw.bgInput,
              border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm, cursor: 'pointer',
            }}
          >Close</button>
        </div>

        <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
          <div style={{
            width: 220, overflowY: 'auto', padding: 8,
            borderRight: `1px solid ${hw.border}`,
          }}>
            {ranked.length === 0 && (
              <div style={{ padding: 16, textAlign: 'center', color: hw.textFaint, fontSize: 11 }}>
                No topics match "{query}"
              </div>
            )}
            {ranked.map(t => {
              const isActive = t.id === active.id
              return (
                <div
                  key={t.id}
                  onClick={() => setActiveId(t.id)}
                  style={{
                    padding: '6px 10px', borderRadius: hw.radius.sm,
                    background: isActive ? hw.accentDim : 'transparent',
                    border: `1px solid ${isActive ? hw.accent : 'transparent'}`,
                    cursor: 'pointer', marginBottom: 2,
                  }}
                >
                  <div style={{
                    fontSize: 11, fontWeight: 600,
                    color: isActive ? hw.accentLight : hw.textPrimary,
                  }}>
                    {t.title}
                  </div>
                  <div style={{ fontSize: 9, color: hw.textFaint, marginTop: 1 }}>
                    {t.panel}
                  </div>
                </div>
              )
            })}
          </div>

          <div style={{ flex: 1, overflowY: 'auto', padding: 20 }}>
            <div style={{ fontSize: 9, color: hw.textFaint, letterSpacing: 0.6, textTransform: 'uppercase' }}>
              {active.panel}
            </div>
            <div style={{ fontSize: 16, fontWeight: 600, color: hw.textPrimary, marginTop: 2 }}>
              {active.title}
            </div>
            <div style={{
              fontSize: 12, color: hw.textSecondary, lineHeight: 1.55, marginTop: 12,
            }}>
              {active.body}
            </div>

            {active.shortcuts && active.shortcuts.length > 0 && (
              <div style={{ marginTop: 20 }}>
                <div style={{
                  fontSize: 9, color: hw.textFaint, letterSpacing: 0.6,
                  textTransform: 'uppercase', marginBottom: 6,
                }}>
                  Shortcuts
                </div>
                {active.shortcuts.map(id => {
                  const action = ACTIONS.find(a => a.id === id)
                  if (!action) return null
                  return (
                    <div
                      key={id}
                      style={{
                        display: 'flex', alignItems: 'center', gap: 12,
                        padding: '4px 6px', borderRadius: hw.radius.sm,
                        background: 'rgba(255,255,255,0.015)', marginBottom: 2,
                      }}
                    >
                      <span style={{
                        fontSize: 10, fontFamily: 'ui-monospace, Menlo, monospace',
                        color: hw.accent, minWidth: 180, padding: '2px 6px',
                        background: hw.bgInput,
                        border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
                        textAlign: 'center',
                      }}>
                        {bindingLabel(bindings[id])}
                      </span>
                      <span style={{ fontSize: 11, color: hw.textSecondary }}>
                        {action.label}
                      </span>
                    </div>
                  )
                })}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}
