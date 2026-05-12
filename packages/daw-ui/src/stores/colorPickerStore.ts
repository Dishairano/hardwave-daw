import { create } from 'zustand'

const RECENT_STORAGE_KEY = 'hardwave.daw.colorPicker.recent'
const RECENT_CAP = 12

/**
 * Owns the open/closed state of the global color picker popover plus a
 * persistent "recent colors" history shared across track + clip colour
 * pickers. The popover itself lives at the App level (see App.tsx
 * `<ColorPicker />`) so it floats above every panel without each panel
 * needing to mount its own copy.
 *
 * `open()` accepts an anchor rect so the popover can position itself
 * relative to the swatch the user clicked. The `onPick` callback is
 * how the caller receives the chosen colour — caller is responsible
 * for persisting it (e.g. `setTrackColor`, `setClipColor`).
 */

export interface ColorPickerOpenArgs {
  /** Position to anchor the popover next to. */
  anchor: DOMRect
  /** Initial colour to seed the picker with (and highlight as active). */
  current?: string | null
  /** Fired on every preset click + on Apply for custom colours. */
  onPick: (color: string) => void
  /** Optional "default" callback — clears the override / resets. */
  onClear?: () => void
  /** Title shown in the popover header. */
  title?: string
}

interface ColorPickerState {
  args: ColorPickerOpenArgs | null
  recent: string[]
  open: (args: ColorPickerOpenArgs) => void
  close: () => void
  /** Persists the colour into the MRU list (and to localStorage). */
  markUsed: (color: string) => void
}

function hydrateRecent(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_STORAGE_KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed.filter((c): c is string => typeof c === 'string').slice(0, RECENT_CAP)
  } catch {
    return []
  }
}

function persistRecent(list: string[]) {
  try {
    localStorage.setItem(RECENT_STORAGE_KEY, JSON.stringify(list.slice(0, RECENT_CAP)))
  } catch {
    /* ignore */
  }
}

export const useColorPickerStore = create<ColorPickerState>((set, get) => ({
  args: null,
  recent: hydrateRecent(),
  open: (args) => set({ args }),
  close: () => set({ args: null }),
  markUsed: (color) => {
    const norm = color.toLowerCase()
    const cur = get().recent
    if (cur[0] === norm) return
    const next = [norm, ...cur.filter((c) => c.toLowerCase() !== norm)].slice(0, RECENT_CAP)
    persistRecent(next)
    set({ recent: next })
  },
}))
