import { create } from 'zustand'

const STORAGE_KEY = 'hardwave.daw.shortcuts'

export interface Binding {
  code: string          // KeyboardEvent.code, e.g. 'KeyN', 'Space', 'F5', 'Slash'
  ctrl?: boolean
  shift?: boolean
  alt?: boolean
}

export type ActionId =
  | 'newProject' | 'openProject' | 'save' | 'saveAs'
  | 'selectAll' | 'duplicate' | 'copy' | 'cut' | 'paste'
  | 'undo' | 'redo'
  | 'togglePlay' | 'deleteSelection' | 'splitClip'
  | 'gotoStart' | 'gotoEnd' | 'toggleLoop'
  | 'togglePlaylist' | 'toggleChannelRack' | 'togglePianoRoll'
  | 'toggleBrowser' | 'toggleMixer' | 'toggleShortcutsPanel'

export interface ActionDef {
  id: ActionId
  label: string
  category: string
}

export const ACTIONS: ActionDef[] = [
  { id: 'newProject',       label: 'New project',               category: 'Project' },
  { id: 'openProject',      label: 'Open project',              category: 'Project' },
  { id: 'save',             label: 'Save project',              category: 'Project' },
  { id: 'saveAs',           label: 'Save project as…',          category: 'Project' },
  { id: 'undo',             label: 'Undo',                       category: 'Project' },
  { id: 'redo',             label: 'Redo',                       category: 'Project' },

  { id: 'selectAll',        label: 'Select all',                 category: 'Editing' },
  { id: 'duplicate',        label: 'Duplicate selection',        category: 'Editing' },
  { id: 'copy',             label: 'Copy',                       category: 'Editing' },
  { id: 'cut',              label: 'Cut',                        category: 'Editing' },
  { id: 'paste',            label: 'Paste at playhead',          category: 'Editing' },
  { id: 'deleteSelection',  label: 'Delete selection',           category: 'Editing' },
  { id: 'splitClip',        label: 'Split clip at edit cursor',  category: 'Editing' },

  { id: 'togglePlay',       label: 'Play / pause',               category: 'Transport' },
  { id: 'gotoStart',        label: 'Return to start',            category: 'Transport' },
  { id: 'gotoEnd',          label: 'Jump to project end',        category: 'Transport' },
  { id: 'toggleLoop',       label: 'Toggle loop region',         category: 'Transport' },

  { id: 'togglePlaylist',       label: 'Toggle Playlist',            category: 'Panels' },
  { id: 'toggleChannelRack',    label: 'Toggle Channel Rack',        category: 'Panels' },
  { id: 'togglePianoRoll',      label: 'Toggle Piano Roll',          category: 'Panels' },
  { id: 'toggleBrowser',        label: 'Toggle Browser',             category: 'Panels' },
  { id: 'toggleMixer',          label: 'Toggle Mixer',               category: 'Panels' },
  { id: 'toggleShortcutsPanel', label: 'Toggle this shortcuts panel', category: 'Panels' },
]

export const DEFAULTS: Record<ActionId, Binding> = {
  newProject:  { code: 'KeyN', ctrl: true },
  openProject: { code: 'KeyO', ctrl: true },
  save:        { code: 'KeyS', ctrl: true },
  saveAs:      { code: 'KeyS', ctrl: true, shift: true },
  selectAll:   { code: 'KeyA', ctrl: true },
  duplicate:   { code: 'KeyD', ctrl: true },
  copy:        { code: 'KeyC', ctrl: true },
  cut:         { code: 'KeyX', ctrl: true },
  paste:       { code: 'KeyV', ctrl: true },
  undo:        { code: 'KeyZ', ctrl: true },
  redo:        { code: 'KeyY', ctrl: true },
  togglePlay:  { code: 'Space' },
  deleteSelection: { code: 'Delete' },
  splitClip:   { code: 'KeyS' },
  gotoStart:   { code: 'Home' },
  gotoEnd:     { code: 'End' },
  toggleLoop:  { code: 'KeyL' },
  togglePlaylist:       { code: 'F5' },
  toggleChannelRack:    { code: 'F6' },
  togglePianoRoll:      { code: 'F7' },
  toggleBrowser:        { code: 'F8' },
  toggleMixer:          { code: 'F9' },
  toggleShortcutsPanel: { code: 'Slash', shift: true },
}

export interface Preset {
  id: string
  name: string
  description: string
  bindings: Partial<Record<ActionId, Binding>>
}

export const PRESETS: Preset[] = [
  {
    id: 'hardwave',
    name: 'Hardwave default',
    description: 'Ships with the DAW — FL-like panel keys',
    bindings: DEFAULTS,
  },
  {
    id: 'flstudio',
    name: 'FL Studio',
    description: 'Matches Image-Line FL Studio defaults',
    bindings: {
      ...DEFAULTS,
      // FL Studio panel keys are identical to our defaults (F5-F9).
      // Classic FL shortcuts:
      togglePlay:  { code: 'Space' },
      toggleLoop:  { code: 'Numpad1' },
      gotoStart:   { code: 'Home' },
    },
  },
  {
    id: 'ableton',
    name: 'Ableton Live',
    description: 'Approximates Ableton Live defaults',
    bindings: {
      ...DEFAULTS,
      togglePlay:       { code: 'Space' },
      gotoStart:        { code: 'Home' },
      toggleBrowser:    { code: 'KeyB', ctrl: true, alt: true },
      togglePianoRoll:  { code: 'KeyM', ctrl: true, alt: true },
      toggleMixer:      { code: 'KeyR', ctrl: true, alt: true },
      togglePlaylist:   { code: 'Tab' },
      toggleChannelRack: { code: 'KeyS', ctrl: true, alt: true },
    },
  },
]

function isBinding(v: unknown): v is Binding {
  return !!v && typeof v === 'object' && typeof (v as Binding).code === 'string'
}

function hydrate(): Record<ActionId, Binding> {
  const result: Record<ActionId, Binding> = { ...DEFAULTS }
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return result
    const parsed = JSON.parse(raw) as Partial<Record<ActionId, Binding>>
    for (const a of ACTIONS) {
      const b = parsed[a.id]
      if (isBinding(b)) result[a.id] = b
    }
  } catch { /* ignore */ }
  return result
}

function persist(bindings: Record<ActionId, Binding>) {
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(bindings)) } catch { /* ignore */ }
}

export function bindingLabel(b: Binding | undefined | null): string {
  if (!b) return '—'
  const parts: string[] = []
  if (b.ctrl) parts.push('Ctrl')
  if (b.alt) parts.push('Alt')
  if (b.shift) parts.push('Shift')
  parts.push(keyLabel(b.code))
  return parts.join(' + ')
}

function keyLabel(code: string): string {
  if (code.startsWith('Key')) return code.slice(3)
  if (code.startsWith('Digit')) return code.slice(5)
  if (code.startsWith('Numpad')) return `Num ${code.slice(6)}`
  switch (code) {
    case 'Space': return 'Space'
    case 'Slash': return '/'
    case 'Backslash': return '\\'
    case 'Comma': return ','
    case 'Period': return '.'
    case 'Semicolon': return ';'
    case 'Quote': return "'"
    case 'BracketLeft': return '['
    case 'BracketRight': return ']'
    case 'Minus': return '-'
    case 'Equal': return '='
    case 'Backquote': return '`'
    case 'ArrowUp': return '↑'
    case 'ArrowDown': return '↓'
    case 'ArrowLeft': return '←'
    case 'ArrowRight': return '→'
    case 'Delete': return 'Delete'
    case 'Backspace': return 'Backspace'
    case 'Enter': return 'Enter'
    case 'Escape': return 'Esc'
    case 'Tab': return 'Tab'
    case 'Home': return 'Home'
    case 'End': return 'End'
    case 'PageUp': return 'PgUp'
    case 'PageDown': return 'PgDn'
    default: return code
  }
}

export function matchesBinding(e: KeyboardEvent, b: Binding): boolean {
  if (e.code !== b.code) return false
  const ctrl = e.ctrlKey || e.metaKey
  if (!!b.ctrl !== ctrl) return false
  if (!!b.shift !== e.shiftKey) return false
  if (!!b.alt !== e.altKey) return false
  return true
}

export function isReservedBinding(b: Binding): boolean {
  // Don't let users rebind bare modifier keys or F5 refresh without modifiers — these are handled fine.
  // No hard reservations today; placeholder for future guards.
  if (!b.code) return true
  return false
}

interface ShortcutsState {
  bindings: Record<ActionId, Binding>
  capturingFor: ActionId | null
  startCapture: (id: ActionId | null) => void
  setBinding: (id: ActionId, b: Binding | null) => void
  resetAll: () => void
  loadPreset: (presetId: string) => void
  exportJson: () => string
  importJson: (json: string) => { ok: boolean; error?: string }
  findConflict: (b: Binding, ignoreId?: ActionId) => ActionId | null
  matchEvent: (e: KeyboardEvent) => ActionId | null
}

export const useShortcutsStore = create<ShortcutsState>((set, get) => ({
  bindings: hydrate(),
  capturingFor: null,

  startCapture: (id) => set({ capturingFor: id }),

  setBinding: (id, b) => {
    const next = { ...get().bindings }
    if (b == null) {
      // Restore default for this action
      next[id] = DEFAULTS[id]
    } else {
      next[id] = b
    }
    persist(next)
    set({ bindings: next, capturingFor: null })
  },

  resetAll: () => {
    const next = { ...DEFAULTS }
    persist(next)
    set({ bindings: next, capturingFor: null })
  },

  loadPreset: (presetId) => {
    const preset = PRESETS.find(p => p.id === presetId)
    if (!preset) return
    const next: Record<ActionId, Binding> = { ...DEFAULTS }
    for (const a of ACTIONS) {
      const b = preset.bindings[a.id]
      if (b) next[a.id] = b
    }
    persist(next)
    set({ bindings: next, capturingFor: null })
  },

  exportJson: () => JSON.stringify(get().bindings, null, 2),

  importJson: (json) => {
    try {
      const parsed = JSON.parse(json) as Partial<Record<ActionId, Binding>>
      const next: Record<ActionId, Binding> = { ...DEFAULTS }
      for (const a of ACTIONS) {
        const b = parsed[a.id]
        if (isBinding(b)) next[a.id] = b
      }
      persist(next)
      set({ bindings: next, capturingFor: null })
      return { ok: true }
    } catch (err) {
      return { ok: false, error: String(err) }
    }
  },

  findConflict: (b, ignoreId) => {
    const bs = get().bindings
    for (const a of ACTIONS) {
      if (a.id === ignoreId) continue
      const other = bs[a.id]
      if (other.code === b.code
        && !!other.ctrl === !!b.ctrl
        && !!other.shift === !!b.shift
        && !!other.alt === !!b.alt) return a.id
    }
    return null
  },

  matchEvent: (e) => {
    const bs = get().bindings
    for (const a of ACTIONS) {
      if (matchesBinding(e, bs[a.id])) return a.id
    }
    return null
  },
}))
