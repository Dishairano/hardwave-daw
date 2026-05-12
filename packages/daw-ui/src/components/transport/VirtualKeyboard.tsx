import { memo, useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'

/**
 * On-screen piano. Two octaves of mouse-and-touch playable keys that
 * inject MIDI events into the engine via `inject_midi_event`, sharing
 * the exact pipeline used by the QWERTY-as-MIDI hook and real hardware
 * controllers.
 *
 * Visual model mirrors FL Studio's on-screen typing keyboard popup:
 * a flat strip of seven white keys per octave with five black keys
 * overlaid on top. The active key gets a highlight so the user knows
 * which note is sounding. Pressing a second key while one is already
 * held releases the prior NoteOff and fires the new NoteOn — the
 * engine's MidiTrackNode is monophonic anyway, so this matches the
 * audio behaviour and avoids stuck notes if the user drags between
 * keys.
 *
 * Two octaves was a deliberate cap: the panel still fits inside the
 * narrowest sidebar slot the layout uses, and an octave-shift control
 * covers any higher / lower range the user actually plays.
 */

const WHITE_KEYS_PER_OCTAVE = 7
const OCTAVES = 2
const TOTAL_WHITES = WHITE_KEYS_PER_OCTAVE * OCTAVES

// Note offsets from the octave's C, by white-key index 0..6.
const WHITE_OFFSETS = [0, 2, 4, 5, 7, 9, 11]
// Black keys defined as (white-key-index-to-the-left, offset-from-C).
const BLACK_OFFSETS: Array<{ leftWhite: number; offset: number }> = [
  { leftWhite: 0, offset: 1 },
  { leftWhite: 1, offset: 3 },
  { leftWhite: 3, offset: 6 },
  { leftWhite: 4, offset: 8 },
  { leftWhite: 5, offset: 10 },
]

const DEFAULT_OCTAVE = 4
const DEFAULT_VELOCITY = 0.78

interface ActiveNote {
  pitch: number
  whiteIndex?: number
  blackIndex?: number
}

export interface VirtualKeyboardProps {
  /** Hide chrome — when false the widget renders nothing. */
  visible: boolean
  onClose: () => void
}

export const VirtualKeyboard = memo(function VirtualKeyboard({
  visible,
  onClose,
}: VirtualKeyboardProps) {
  const [octave, setOctave] = useState(DEFAULT_OCTAVE)
  const [active, setActive] = useState<ActiveNote | null>(null)
  const activeRef = useRef<ActiveNote | null>(null)
  activeRef.current = active

  const sendOn = useCallback((pitch: number) => {
    void invoke('inject_midi_event', {
      event: { kind: 'note_on', channel: 0, note: pitch, velocity: DEFAULT_VELOCITY },
    })
  }, [])
  const sendOff = useCallback((pitch: number) => {
    void invoke('inject_midi_event', {
      event: { kind: 'note_off', channel: 0, note: pitch },
    })
  }, [])

  const press = useCallback(
    (next: ActiveNote) => {
      const prev = activeRef.current
      if (prev && prev.pitch === next.pitch) return
      if (prev) sendOff(prev.pitch)
      sendOn(next.pitch)
      setActive(next)
    },
    [sendOn, sendOff],
  )

  const release = useCallback(() => {
    const cur = activeRef.current
    if (!cur) return
    sendOff(cur.pitch)
    setActive(null)
  }, [sendOff])

  // Global mouseup catches releases that happen outside the key
  // element (e.g. user dragged off the panel while holding).
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

  const baseC = (octave + 1) * 12 // C{octave} as MIDI pitch (C4 = 60)

  // White-key bounds for absolute-positioning the black keys on top.
  const whiteW = 100 / TOTAL_WHITES // %
  const blackW = whiteW * 0.6

  return (
    <div className="hw-vkbd">
      <div className="hw-vkbd-titlebar">
        <span className="hw-vkbd-title">Keyboard</span>
        <div className="hw-vkbd-octave">
          <button
            type="button"
            className="hw-vkbd-octave-btn"
            onClick={() => setOctave((o) => Math.max(0, o - 1))}
            aria-label="Octave down"
          >
            −
          </button>
          <span className="hw-vkbd-octave-label">C{octave}</span>
          <button
            type="button"
            className="hw-vkbd-octave-btn"
            onClick={() => setOctave((o) => Math.min(9, o + 1))}
            aria-label="Octave up"
          >
            +
          </button>
        </div>
        <button
          type="button"
          className="hw-vkbd-close"
          onClick={onClose}
          aria-label="Close keyboard"
        >
          ×
        </button>
      </div>
      <div
        className="hw-vkbd-keys"
        onMouseLeave={release}
        role="application"
        aria-label="On-screen MIDI keyboard"
      >
        {/* White keys */}
        {Array.from({ length: TOTAL_WHITES }).map((_, i) => {
          const octaveIdx = Math.floor(i / WHITE_KEYS_PER_OCTAVE)
          const whiteInOctave = i % WHITE_KEYS_PER_OCTAVE
          const pitch = baseC + octaveIdx * 12 + WHITE_OFFSETS[whiteInOctave]
          const isActive = active?.whiteIndex === i
          return (
            <button
              key={`w-${i}`}
              type="button"
              className={`hw-vkbd-white${isActive ? ' is-active' : ''}`}
              style={{ left: `${i * whiteW}%`, width: `${whiteW}%` }}
              onMouseDown={(e) => {
                e.preventDefault()
                press({ pitch, whiteIndex: i })
              }}
              onMouseEnter={(e) => {
                if (e.buttons === 1) press({ pitch, whiteIndex: i })
              }}
              onTouchStart={(e) => {
                e.preventDefault()
                press({ pitch, whiteIndex: i })
              }}
              aria-label={`MIDI note ${pitch}`}
            >
              {whiteInOctave === 0 ? <span className="hw-vkbd-keylabel">C{octave + octaveIdx}</span> : null}
            </button>
          )
        })}
        {/* Black keys */}
        {Array.from({ length: OCTAVES }).flatMap((_, octaveIdx) =>
          BLACK_OFFSETS.map((b) => {
            const whiteI = octaveIdx * WHITE_KEYS_PER_OCTAVE + b.leftWhite
            // Black key sits at the boundary between whiteI and whiteI+1.
            const leftPct = (whiteI + 1) * whiteW - blackW / 2
            const pitch = baseC + octaveIdx * 12 + b.offset
            const id = `${octaveIdx}-${b.offset}`
            const isActive = active?.blackIndex === id.charCodeAt(0)
            return (
              <button
                key={`b-${id}`}
                type="button"
                className={`hw-vkbd-black${isActive ? ' is-active' : ''}`}
                style={{ left: `${leftPct}%`, width: `${blackW}%` }}
                onMouseDown={(e) => {
                  e.preventDefault()
                  press({ pitch, blackIndex: id.charCodeAt(0) })
                }}
                onMouseEnter={(e) => {
                  if (e.buttons === 1) press({ pitch, blackIndex: id.charCodeAt(0) })
                }}
                onTouchStart={(e) => {
                  e.preventDefault()
                  press({ pitch, blackIndex: id.charCodeAt(0) })
                }}
                aria-label={`MIDI note ${pitch}`}
              />
            )
          }),
        )}
      </div>
    </div>
  )
})
