import { memo, useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { defaultPadNote, useTouchControllerStore } from '../../stores/touchControllerStore'
import { pitchToName as namePitch } from '../../stores/generalPrefsStore'

/**
 * Touch Controllers — virtual on-screen MIDI keyboard + drum-pad
 * widget. FL Studio parity for the panel accessed via View menu →
 * Touch controller (Alt+F7).
 *
 * Two modes:
 *
 *  - **Keyboard** — single or double row of piano keys (white + black
 *    overlay), with root-note transpose, velocity from vertical play
 *    position, octave shift, optional pitch-name labels.
 *  - **Drumpad** — grid (2×1 … 16×8) of velocity-sensitive pads, each
 *    with a customisable MIDI note and colour.
 *
 * Routing is identical to the QWERTY-as-MIDI hook + hardware
 * controllers: every press fires `inject_midi_event`, which the audio
 * thread drains alongside real controller events. Performances land in
 * the capture ring, so the same `commit_recording_to_midi_clip` flow
 * works whether you play with hardware, the typing keyboard, or this
 * widget.
 *
 * All visible settings (mode, rows, root note, velocity-from-position,
 * labels, scrollbar lock, pad grid + per-pad assignments) live in
 * `touchControllerStore` and persist across sessions.
 */

const WHITE_KEYS_PER_OCTAVE = 7
const KEYBOARD_OCTAVES = 2 // span — root note shifts what each white key plays
const TOTAL_WHITES = WHITE_KEYS_PER_OCTAVE * KEYBOARD_OCTAVES
const WHITE_OFFSETS = [0, 2, 4, 5, 7, 9, 11]
const BLACK_OFFSETS: Array<{ leftWhite: number; offset: number }> = [
  { leftWhite: 0, offset: 1 },
  { leftWhite: 1, offset: 3 },
  { leftWhite: 3, offset: 6 },
  { leftWhite: 4, offset: 8 },
  { leftWhite: 5, offset: 10 },
]
const DEFAULT_VELOCITY = 0.78
const MIN_VELOCITY = 0.1
// Pitch labels honour the System Settings → General "Note naming"
// convention (English / Germanic / Solfège). Delegated to the
// global generalPrefsStore so labels stay in sync across PianoRoll,
// Touch Controller, Browser sample tags, etc.
function pitchToName(pitch: number): string {
  return namePitch(pitch)
}

interface ActiveNote {
  pitch: number
  origin: string
}

export interface VirtualKeyboardProps {
  /** When false the widget renders nothing. Wired to the store's
   * `visible` flag from the App layer so any external control point
   * (View menu, Alt+F7, the store directly) toggles in sync. */
  visible: boolean
  onClose: () => void
}

export const VirtualKeyboard = memo(function VirtualKeyboard({
  visible,
  onClose,
}: VirtualKeyboardProps) {
  const settings = useTouchControllerStore()
  const [active, setActive] = useState<ActiveNote | null>(null)
  const [showSettings, setShowSettings] = useState(false)
  const [selectedPads, setSelectedPads] = useState<Set<string>>(new Set())
  const activeRef = useRef<ActiveNote | null>(null)
  activeRef.current = active

  const sendOn = useCallback((pitch: number, velocity: number) => {
    void invoke('inject_midi_event', {
      event: { kind: 'note_on', channel: 0, note: pitch, velocity },
    })
  }, [])
  const sendOff = useCallback((pitch: number) => {
    void invoke('inject_midi_event', {
      event: { kind: 'note_off', channel: 0, note: pitch },
    })
  }, [])

  const press = useCallback(
    (pitch: number, origin: string, velocity: number) => {
      const prev = activeRef.current
      if (prev && prev.pitch === pitch) return
      if (prev) sendOff(prev.pitch)
      sendOn(pitch, velocity)
      setActive({ pitch, origin })
    },
    [sendOn, sendOff],
  )

  const release = useCallback(() => {
    const cur = activeRef.current
    if (!cur) return
    sendOff(cur.pitch)
    setActive(null)
  }, [sendOff])

  useEffect(() => {
    if (!visible) return
    const onUp = () => release()
    window.addEventListener('mouseup', onUp)
    window.addEventListener('touchend', onUp)
    window.addEventListener('blur', onUp)
    return () => {
      window.removeEventListener('mouseup', onUp)
      window.removeEventListener('touchend', onUp)
      window.removeEventListener('blur', onUp)
    }
  }, [visible, release])

  if (!visible) return null

  // Velocity derived from vertical mouse / first-touch position within
  // the target. Position 0 (top edge) → MIN_VELOCITY, position 1
  // (bottom edge) → 1.0. Accepts either React.MouseEvent or
  // React.TouchEvent — TouchEvent has no top-level `clientY` so we
  // pull the value from the first changed touch, falling back to the
  // first active touch in case the event has already settled.
  const computeVelocity = (
    e: React.MouseEvent | React.TouchEvent,
    rect: DOMRect,
  ): number => {
    if (!settings.velocityFromPosition) return DEFAULT_VELOCITY
    const clientY = 'clientY' in e
      ? e.clientY
      : (e.changedTouches[0]?.clientY ?? e.touches[0]?.clientY ?? rect.top)
    const ratio = (clientY - rect.top) / Math.max(1, rect.height)
    return Math.max(MIN_VELOCITY, Math.min(1, ratio))
  }

  return (
    <div className="hw-vkbd">
      <div className="hw-vkbd-titlebar">
        <span className="hw-vkbd-title">
          {settings.mode === 'keyboard' ? 'Touch Keyboard' : 'Touch Pads'}
        </span>
        <div className="hw-vkbd-mode-toggle">
          <button
            type="button"
            className={`hw-vkbd-mode-btn${settings.mode === 'keyboard' ? ' is-active' : ''}`}
            onClick={() => settings.setMode('keyboard')}
          >
            Keys
          </button>
          <button
            type="button"
            className={`hw-vkbd-mode-btn${settings.mode === 'drumpad' ? ' is-active' : ''}`}
            onClick={() => settings.setMode('drumpad')}
          >
            Pads
          </button>
        </div>
        {settings.mode === 'keyboard' && (
          <div className="hw-vkbd-octave">
            <button
              type="button"
              className="hw-vkbd-octave-btn"
              onClick={() => settings.setRootNote(settings.rootNote - 12)}
              aria-label="Octave down"
            >
              −
            </button>
            <span className="hw-vkbd-octave-label">{pitchToName(settings.rootNote)}</span>
            <button
              type="button"
              className="hw-vkbd-octave-btn"
              onClick={() => settings.setRootNote(settings.rootNote + 12)}
              aria-label="Octave up"
            >
              +
            </button>
          </div>
        )}
        <button
          type="button"
          className="hw-vkbd-gear"
          onClick={() => setShowSettings((v) => !v)}
          aria-label="Settings"
          title="Options"
        >
          ⚙
        </button>
        <button
          type="button"
          className="hw-vkbd-close"
          onClick={onClose}
          aria-label="Close"
        >
          ×
        </button>
      </div>

      {showSettings && (
        <div className="hw-vkbd-settings">
          {settings.mode === 'keyboard' && (
            <label className="hw-vkbd-row">
              <span>Keyboard rows</span>
              <select
                value={settings.keyboardRows}
                onChange={(e) => settings.setKeyboardRows(Number(e.target.value) as 1 | 2)}
              >
                <option value={1}>Single</option>
                <option value={2}>Double</option>
              </select>
            </label>
          )}
          {settings.mode === 'drumpad' && (
            <>
              <label className="hw-vkbd-row">
                <span>Pad columns</span>
                <input
                  type="number"
                  min={2}
                  max={16}
                  value={settings.padGridCols}
                  onChange={(e) => settings.setPadGrid(Number(e.target.value), settings.padGridRows)}
                />
              </label>
              <label className="hw-vkbd-row">
                <span>Pad rows</span>
                <input
                  type="number"
                  min={1}
                  max={8}
                  value={settings.padGridRows}
                  onChange={(e) => settings.setPadGrid(settings.padGridCols, Number(e.target.value))}
                />
              </label>
              {selectedPads.size > 0 && (
                <div className="hw-vkbd-row">
                  <span>{selectedPads.size} pad{selectedPads.size === 1 ? '' : 's'} selected</span>
                  <div className="hw-vkbd-color-swatches">
                    {['#ff2d4f', '#a872ff', '#5a9bff', '#3ed07a', '#f0a032', null].map((c) => (
                      <button
                        key={c ?? 'reset'}
                        type="button"
                        className="hw-vkbd-color-swatch"
                        style={{ background: c ?? 'transparent', border: c ? '0' : '1px dashed #555' }}
                        title={c ?? 'Reset'}
                        onClick={() => {
                          const cells = Array.from(selectedPads).map((k) => {
                            const [row, col] = k.split(',').map(Number)
                            return { row, col }
                          })
                          settings.setPadColors(cells, c)
                          setSelectedPads(new Set())
                        }}
                      />
                    ))}
                  </div>
                </div>
              )}
            </>
          )}
          <label className="hw-vkbd-row">
            <span>Velocity from position</span>
            <input
              type="checkbox"
              checked={settings.velocityFromPosition}
              onChange={(e) => settings.setVelocityFromPosition(e.target.checked)}
            />
          </label>
          <label className="hw-vkbd-row">
            <span>Show note labels</span>
            <input
              type="checkbox"
              checked={settings.showNoteLabels}
              onChange={(e) => settings.setShowNoteLabels(e.target.checked)}
            />
          </label>
          <label className="hw-vkbd-row">
            <span>Lock scrollbar</span>
            <input
              type="checkbox"
              checked={settings.scrollbarLocked}
              onChange={(e) => settings.setScrollbarLocked(e.target.checked)}
            />
          </label>
        </div>
      )}

      {settings.mode === 'keyboard' ? (
        <KeyboardBody
          rootNote={settings.rootNote}
          rows={settings.keyboardRows}
          showLabels={settings.showNoteLabels}
          activePitch={active?.pitch ?? null}
          setRootNote={settings.setRootNote}
          press={(pitch, e) => {
            const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
            press(pitch, 'key', computeVelocity(e, rect))
          }}
          release={release}
        />
      ) : (
        <DrumPadBody
          cols={settings.padGridCols}
          rows={settings.padGridRows}
          pads={settings.pads}
          rootNote={settings.rootNote}
          showLabels={settings.showNoteLabels}
          selectedPads={selectedPads}
          activePitch={active?.pitch ?? null}
          press={(pitch, e) => {
            const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
            press(pitch, 'pad', computeVelocity(e, rect))
          }}
          release={release}
          toggleSelection={(row, col) => {
            const key = `${row},${col}`
            setSelectedPads((prev) => {
              const next = new Set(prev)
              if (next.has(key)) next.delete(key)
              else next.add(key)
              return next
            })
          }}
        />
      )}
    </div>
  )
})

interface KeyboardBodyProps {
  rootNote: number
  rows: 1 | 2
  showLabels: boolean
  activePitch: number | null
  setRootNote: (n: number) => void
  press: (pitch: number, e: React.MouseEvent | React.TouchEvent) => void
  release: () => void
}

function KeyboardBody({
  rootNote,
  rows,
  showLabels,
  activePitch,
  setRootNote,
  press,
  release,
}: KeyboardBodyProps) {
  // The visible left-edge is the C at-or-below rootNote.
  const baseC = rootNote - (rootNote % 12)
  const whiteW = 100 / TOTAL_WHITES
  const blackW = whiteW * 0.6

  const renderRow = (rowIdx: number) => {
    const rowBase = baseC + rowIdx * KEYBOARD_OCTAVES * 12
    return (
      <div className="hw-vkbd-keys" onMouseLeave={release} key={`row-${rowIdx}`}>
        {Array.from({ length: TOTAL_WHITES }).map((_, i) => {
          const octaveIdx = Math.floor(i / WHITE_KEYS_PER_OCTAVE)
          const whiteInOctave = i % WHITE_KEYS_PER_OCTAVE
          const pitch = rowBase + octaveIdx * 12 + WHITE_OFFSETS[whiteInOctave]
          const isActive = activePitch === pitch
          const labelOctave = Math.floor(pitch / 12) - 1
          return (
            <button
              key={`w-${rowIdx}-${i}`}
              type="button"
              className={`hw-vkbd-white${isActive ? ' is-active' : ''}`}
              style={{ left: `${i * whiteW}%`, width: `${whiteW}%` }}
              onMouseDown={(e) => {
                e.preventDefault()
                press(pitch, e)
              }}
              onMouseEnter={(e) => {
                if (e.buttons === 1) press(pitch, e)
              }}
              onTouchStart={(e) => {
                e.preventDefault()
                press(pitch, e)
              }}
              onContextMenu={(e) => {
                e.preventDefault()
                setRootNote(pitch)
              }}
              aria-label={`MIDI note ${pitch}`}
              title="Right-click to set root note"
            >
              {(showLabels || whiteInOctave === 0) && (
                <span className="hw-vkbd-keylabel">
                  {showLabels ? pitchToName(pitch) : `C${labelOctave}`}
                </span>
              )}
            </button>
          )
        })}
        {Array.from({ length: KEYBOARD_OCTAVES }).flatMap((_, octaveIdx) =>
          BLACK_OFFSETS.map((b) => {
            const whiteI = octaveIdx * WHITE_KEYS_PER_OCTAVE + b.leftWhite
            const leftPct = (whiteI + 1) * whiteW - blackW / 2
            const pitch = rowBase + octaveIdx * 12 + b.offset
            const isActive = activePitch === pitch
            return (
              <button
                key={`b-${rowIdx}-${octaveIdx}-${b.offset}`}
                type="button"
                className={`hw-vkbd-black${isActive ? ' is-active' : ''}`}
                style={{ left: `${leftPct}%`, width: `${blackW}%` }}
                onMouseDown={(e) => {
                  e.preventDefault()
                  press(pitch, e)
                }}
                onMouseEnter={(e) => {
                  if (e.buttons === 1) press(pitch, e)
                }}
                onTouchStart={(e) => {
                  e.preventDefault()
                  press(pitch, e)
                }}
                onContextMenu={(e) => {
                  e.preventDefault()
                  setRootNote(pitch)
                }}
                aria-label={`MIDI note ${pitch}`}
              >
                {showLabels ? (
                  <span className="hw-vkbd-blacklabel">{pitchToName(pitch)}</span>
                ) : null}
              </button>
            )
          }),
        )}
      </div>
    )
  }

  return (
    <div className="hw-vkbd-keyboard-body">
      {rows === 2 ? renderRow(1) : null}
      {renderRow(0)}
    </div>
  )
}

interface DrumPadBodyProps {
  cols: number
  rows: number
  pads: Record<string, { note: number; color: string | null }>
  rootNote: number
  showLabels: boolean
  selectedPads: Set<string>
  activePitch: number | null
  press: (pitch: number, e: React.MouseEvent | React.TouchEvent) => void
  release: () => void
  toggleSelection: (row: number, col: number) => void
}

function DrumPadBody({
  cols,
  rows,
  pads,
  rootNote,
  showLabels,
  selectedPads,
  activePitch,
  press,
  release,
  toggleSelection,
}: DrumPadBodyProps) {
  return (
    <div
      className="hw-vkbd-pads"
      onMouseLeave={release}
      style={{
        gridTemplateColumns: `repeat(${cols}, 1fr)`,
        gridTemplateRows: `repeat(${rows}, 1fr)`,
      }}
    >
      {Array.from({ length: rows }).flatMap((_, row) =>
        Array.from({ length: cols }).map((_, col) => {
          const key = `${row},${col}`
          const cfg = pads[key]
          const note = cfg?.note ?? defaultPadNote(row, col, rootNote, cols)
          const color = cfg?.color ?? null
          const isActive = activePitch === note
          const isSelected = selectedPads.has(key)
          return (
            <button
              key={`pad-${key}`}
              type="button"
              className={`hw-vkbd-pad${isActive ? ' is-active' : ''}${
                isSelected ? ' is-selected' : ''
              }`}
              style={color ? { background: color, borderColor: color } : undefined}
              onMouseDown={(e) => {
                if (e.shiftKey) {
                  e.preventDefault()
                  toggleSelection(row, col)
                  return
                }
                e.preventDefault()
                press(note, e)
              }}
              onTouchStart={(e) => {
                e.preventDefault()
                press(note, e)
              }}
              onContextMenu={(e) => {
                e.preventDefault()
                toggleSelection(row, col)
              }}
              aria-label={`Pad ${row},${col} — MIDI note ${note}`}
              title="Shift+click or right-click to select for bulk colour"
            >
              {showLabels ? (
                <span className="hw-vkbd-padlabel">{pitchToName(note)}</span>
              ) : null}
            </button>
          )
        }),
      )}
    </div>
  )
}
