import { useEffect, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'

/**
 * Maps a QWERTY keyboard to MIDI note input, FL Studio / Ableton style.
 *
 * Two rows of piano keys land on top of the alpha grid:
 *
 *   Lower octave (Z–M row, white keys; S D G H J, black keys):
 *     Z=C  S=C# X=D  D=D# C=E  V=F  G=F# B=G  H=G# N=A  J=A# M=B
 *     ,=C+1 L=C#+1 .=D+1 ;=D#+1 /=E+1
 *
 *   Upper octave (Q–U row, white keys; 2 3 5 6 7, black keys):
 *     Q=C+1 2=C#+1 W=D+1 3=D#+1 E=E+1 R=F+1 5=F#+1 T=G+1 6=G#+1
 *     Y=A+1 7=A#+1 U=B+1
 *
 * Middle C lands at MIDI pitch 60 (C4) when `octave` is 4. The current
 * octave is shifted with `[` / `]` and persisted across renders via
 * the octave ref.
 *
 * Routing: every NoteOn/NoteOff goes through the `inject_midi_event`
 * Tauri command, which pushes into the same shared queue as midir
 * hardware callbacks. From there the engine drains it each audio
 * block and routes to MIDI tracks (or to armed audio tracks running
 * an instrument plug-in) per the `accepts_live_midi` flag set during
 * graph rebuild.
 *
 * Active-element guard: a keypress while a text input or dropdown is
 * focused is ignored — otherwise typing a project name would smash
 * notes into the engine.
 *
 * Auto-repeat suppression: holding a key spams keydown events; we
 * track held codes in a Set so the second+ repeated keydowns don't
 * each fire a NoteOn (which would burst a chord of identical voices).
 */

// Lower row layout (z…/), value = offset from current octave's C.
const LOWER: Record<string, number> = {
  KeyZ: 0,
  KeyS: 1,
  KeyX: 2,
  KeyD: 3,
  KeyC: 4,
  KeyV: 5,
  KeyG: 6,
  KeyB: 7,
  KeyH: 8,
  KeyN: 9,
  KeyJ: 10,
  KeyM: 11,
  Comma: 12,
  KeyL: 13,
  Period: 14,
  Semicolon: 15,
  Slash: 16,
}

// Upper row layout (q…u), value = offset from one octave above the
// current octave's C (so it stacks naturally on top of the lower row).
const UPPER: Record<string, number> = {
  KeyQ: 12,
  Digit2: 13,
  KeyW: 14,
  Digit3: 15,
  KeyE: 16,
  KeyR: 17,
  Digit5: 18,
  KeyT: 19,
  Digit6: 20,
  KeyY: 21,
  Digit7: 22,
  KeyU: 23,
}

const DEFAULT_OCTAVE = 4

export interface ComputerMidiKeyboardOptions {
  /** When false the hook installs no listeners. Used to disable input
   * while a focus-stealing modal is up or when the user has explicitly
   * turned the typing keyboard off. */
  enabled: boolean
  /** Velocity used for every NoteOn — typing has no velocity. 0.78 is
   * the FL Studio default (~100 of 127). */
  velocity?: number
}

export function useComputerMidiKeyboard({
  enabled,
  velocity = 0.78,
}: ComputerMidiKeyboardOptions): void {
  const octaveRef = useRef(DEFAULT_OCTAVE)
  const heldRef = useRef<Set<string>>(new Set())

  useEffect(() => {
    if (!enabled) return

    const sendNoteOn = (note: number) => {
      void invoke('inject_midi_event', {
        event: { kind: 'note_on', channel: 0, note, velocity },
      })
    }
    const sendNoteOff = (note: number) => {
      void invoke('inject_midi_event', {
        event: { kind: 'note_off', channel: 0, note },
      })
    }

    const pitchFor = (code: string): number | null => {
      const lo = LOWER[code]
      if (lo !== undefined) {
        return clampPitch(octaveRef.current * 12 + 12 + lo)
      }
      const up = UPPER[code]
      if (up !== undefined) {
        return clampPitch(octaveRef.current * 12 + 12 + up)
      }
      return null
    }

    const onKeyDown = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement | null)?.tagName
      if (
        tag === 'INPUT' ||
        tag === 'TEXTAREA' ||
        tag === 'SELECT' ||
        (e.target as HTMLElement | null)?.isContentEditable
      ) {
        return
      }
      if (e.ctrlKey || e.metaKey || e.altKey) return

      // Octave shift. [ steps down, ] steps up. Clamp 0–9 — the lower
      // row at octave 0 still produces pitches in the audible MIDI
      // range (12..28) and octave 9 covers the top of the keyboard.
      if (e.code === 'BracketLeft') {
        e.preventDefault()
        octaveRef.current = Math.max(0, octaveRef.current - 1)
        return
      }
      if (e.code === 'BracketRight') {
        e.preventDefault()
        octaveRef.current = Math.min(9, octaveRef.current + 1)
        return
      }

      const pitch = pitchFor(e.code)
      if (pitch == null) return
      if (e.repeat || heldRef.current.has(e.code)) {
        e.preventDefault()
        return
      }
      e.preventDefault()
      heldRef.current.add(e.code)
      sendNoteOn(pitch)
    }

    const onKeyUp = (e: KeyboardEvent) => {
      if (!heldRef.current.has(e.code)) return
      const pitch = pitchFor(e.code)
      heldRef.current.delete(e.code)
      if (pitch != null) sendNoteOff(pitch)
    }

    // Window blur (alt-tab) leaves any held key "stuck on" — flush
    // every active NoteOn so the synth doesn't drone after focus loss.
    const onBlur = () => {
      for (const code of heldRef.current) {
        const pitch = pitchFor(code)
        if (pitch != null) sendNoteOff(pitch)
      }
      heldRef.current.clear()
    }

    window.addEventListener('keydown', onKeyDown)
    window.addEventListener('keyup', onKeyUp)
    window.addEventListener('blur', onBlur)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
      window.removeEventListener('keyup', onKeyUp)
      window.removeEventListener('blur', onBlur)
      onBlur()
    }
  }, [enabled, velocity])
}

function clampPitch(p: number): number {
  if (p < 0) return 0
  if (p > 127) return 127
  return p
}
