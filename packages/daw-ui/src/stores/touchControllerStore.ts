import { create } from 'zustand'
import { persist } from 'zustand/middleware'

/**
 * Persistent settings for the Touch Controller widget — the on-screen
 * MIDI keyboard / drum-pad accessed via View menu or Alt+F7.
 *
 * Mirrors FL Studio's Touch Controllers options pane (the gear icon
 * in the panel's title bar) so user expectations carry over: mode
 * toggle (keyboard ↔ drumpad), single vs double keyboard rows, note
 * labels, velocity from vertical play position, scrollbar lock, root
 * note transpose, and drum-pad grid sizing with per-pad MIDI key +
 * colour customisation.
 *
 * State is persisted across sessions (localStorage) — the widget
 * remembers your custom drum-pad layout, root note shift, and colour
 * choices the next time you open Hardwave.
 */

export type TouchControllerMode = 'keyboard' | 'drumpad'

/** Drum-pad grid bounds match the FL Touch Controllers reference. */
export const PAD_GRID_BOUNDS = {
  cols: { min: 2, max: 16 },
  rows: { min: 1, max: 8 },
}

interface PadConfig {
  /** MIDI note number this pad plays. Defaults laid out in C major
   * chromatic starting at root + (row * cols + col). */
  note: number
  /** Hex colour. Defaults to the brand accent — null means "no override". */
  color: string | null
}

interface TouchControllerState {
  /** Whether the Touch Controllers panel is currently visible. The
   * Alt+F7 shortcut + Tools menu toggle this. Persisted so the panel
   * survives reload (FL behaviour). */
  visible: boolean

  mode: TouchControllerMode

  /** Single (1) or double (2) keyboard layout — FL exposes both. */
  keyboardRows: 1 | 2

  /** MIDI note that the leftmost C maps to. FL default is C5 = 72.
   * Right-click on any key in the widget should call setRootNote with
   * that key's pitch. */
  rootNote: number

  /** When true, the vertical position of mouse-down within the key /
   * pad bounds scales velocity from 0.1 (top) to 1.0 (bottom). When
   * false, every press uses a fixed default velocity. */
  velocityFromPosition: boolean

  /** Toggles pitch-name overlays on every white key / pad. */
  showNoteLabels: boolean

  /** Locks the horizontal scrollbar against accidental nudge. The
   * widget hides scroll affordance entirely when true. */
  scrollbarLocked: boolean

  /** Drum-pad grid dimensions. */
  padGridCols: number
  padGridRows: number

  /** Per-pad config keyed by `"row,col"`. Missing entries fall back to
   * note = baseRoot + row*cols + col, color = brand accent. */
  pads: Record<string, PadConfig>

  // ── actions ──
  setVisible: (v: boolean) => void
  toggleVisible: () => void
  setMode: (m: TouchControllerMode) => void
  setKeyboardRows: (r: 1 | 2) => void
  setRootNote: (note: number) => void
  setVelocityFromPosition: (v: boolean) => void
  setShowNoteLabels: (v: boolean) => void
  setScrollbarLocked: (v: boolean) => void
  setPadGrid: (cols: number, rows: number) => void
  setPadNote: (row: number, col: number, note: number) => void
  setPadColor: (row: number, col: number, color: string | null) => void
  setPadColors: (cells: Array<{ row: number; col: number }>, color: string | null) => void
}

export const useTouchControllerStore = create<TouchControllerState>()(
  persist(
    (set) => ({
      visible: false,
      mode: 'keyboard',
      keyboardRows: 1,
      // C5 = 72 matches FL's documented default.
      rootNote: 72,
      velocityFromPosition: true,
      showNoteLabels: false,
      scrollbarLocked: false,
      padGridCols: 4,
      padGridRows: 4,
      pads: {},

      setVisible: (visible) => set({ visible }),
      toggleVisible: () => set((s) => ({ visible: !s.visible })),
      setMode: (mode) => set({ mode }),
      setKeyboardRows: (keyboardRows) => set({ keyboardRows }),
      setRootNote: (rootNote) => set({ rootNote: clampPitch(rootNote) }),
      setVelocityFromPosition: (velocityFromPosition) => set({ velocityFromPosition }),
      setShowNoteLabels: (showNoteLabels) => set({ showNoteLabels }),
      setScrollbarLocked: (scrollbarLocked) => set({ scrollbarLocked }),
      setPadGrid: (cols, rows) =>
        set({
          padGridCols: clamp(cols, PAD_GRID_BOUNDS.cols.min, PAD_GRID_BOUNDS.cols.max),
          padGridRows: clamp(rows, PAD_GRID_BOUNDS.rows.min, PAD_GRID_BOUNDS.rows.max),
        }),
      setPadNote: (row, col, note) =>
        set((s) => ({
          pads: {
            ...s.pads,
            [`${row},${col}`]: { ...(s.pads[`${row},${col}`] ?? { color: null }), note: clampPitch(note) },
          },
        })),
      setPadColor: (row, col, color) =>
        set((s) => ({
          pads: {
            ...s.pads,
            [`${row},${col}`]: { ...(s.pads[`${row},${col}`] ?? { note: 36 }), color },
          },
        })),
      setPadColors: (cells, color) =>
        set((s) => {
          const next = { ...s.pads }
          for (const { row, col } of cells) {
            const key = `${row},${col}`
            next[key] = { ...(next[key] ?? { note: 36 }), color }
          }
          return { pads: next }
        }),
    }),
    { name: 'hw-touch-controller' },
  ),
)

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, Math.round(v)))
}
function clampPitch(p: number): number {
  return clamp(p, 0, 127)
}

/** Default MIDI note for a pad at (row, col) when no explicit override
 * is stored. Row-major: bottom-left starts at root, walks right then
 * up by one octave per row. Mirrors the FL drum-pad default layout
 * (kick + snare + hats on the bottom row). */
export function defaultPadNote(row: number, col: number, root: number, cols: number): number {
  // Bottom row = lowest pitches; row 0 visually rendered at top.
  const fromBottom = 0 // we keep top-row=0 indexing and add upward
  // Simple: row * cols + col offset from root, clamped to MIDI range.
  const idx = row * cols + col + fromBottom
  let note = root + idx
  if (note > 127) note = 127
  if (note < 0) note = 0
  return note
}
