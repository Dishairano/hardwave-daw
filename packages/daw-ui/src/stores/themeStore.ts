import { create } from 'zustand'

export const STORAGE_KEY = 'hardwave.daw.theme'
export const CUSTOM_KEY = 'hardwave.daw.theme.customs'
export const CUSTOM_BG_KEY = 'hardwave.daw.theme.customBg'

export interface ThemePalette {
  id: string
  name: string
  description: string
  accent: string
  accentLight: string
  accentDim: string
  accentGlow: string
  secondary: string
  secondaryDim: string
  selection: string
  selectionDim: string
  glowRed: string
  custom?: boolean
}

export const THEME_PRESETS: ThemePalette[] = [
  {
    id: 'hardwaveRed',
    name: 'Hardwave Red',
    description: 'Classic Hardwave signature red',
    accent: '#DC2626',
    accentLight: '#EF4444',
    accentDim: 'rgba(220,38,38,0.15)',
    accentGlow: 'rgba(220,38,38,0.3)',
    secondary: '#B91C1C',
    secondaryDim: 'rgba(185,28,28,0.15)',
    selection: '#DC2626',
    selectionDim: 'rgba(220,38,38,0.12)',
    glowRed: '0 0 40px rgba(220,38,38,0.08), 0 0 80px rgba(220,38,38,0.04)',
  },
  {
    id: 'midnightBlue',
    name: 'Midnight Blue',
    description: 'Deep azure — late-night studio vibes',
    accent: '#3B82F6',
    accentLight: '#60A5FA',
    accentDim: 'rgba(59,130,246,0.15)',
    accentGlow: 'rgba(59,130,246,0.3)',
    secondary: '#2563EB',
    secondaryDim: 'rgba(37,99,235,0.15)',
    selection: '#3B82F6',
    selectionDim: 'rgba(59,130,246,0.12)',
    glowRed: '0 0 40px rgba(59,130,246,0.08), 0 0 80px rgba(59,130,246,0.04)',
  },
  {
    id: 'forestGreen',
    name: 'Forest Green',
    description: 'Emerald — easy on the eyes for long sessions',
    accent: '#10B981',
    accentLight: '#34D399',
    accentDim: 'rgba(16,185,129,0.15)',
    accentGlow: 'rgba(16,185,129,0.3)',
    secondary: '#059669',
    secondaryDim: 'rgba(5,150,105,0.15)',
    selection: '#10B981',
    selectionDim: 'rgba(16,185,129,0.12)',
    glowRed: '0 0 40px rgba(16,185,129,0.08), 0 0 80px rgba(16,185,129,0.04)',
  },
  {
    id: 'sunsetOrange',
    name: 'Sunset Orange',
    description: 'Warm amber glow',
    accent: '#F97316',
    accentLight: '#FB923C',
    accentDim: 'rgba(249,115,22,0.15)',
    accentGlow: 'rgba(249,115,22,0.3)',
    secondary: '#EA580C',
    secondaryDim: 'rgba(234,88,12,0.15)',
    selection: '#F97316',
    selectionDim: 'rgba(249,115,22,0.12)',
    glowRed: '0 0 40px rgba(249,115,22,0.08), 0 0 80px rgba(249,115,22,0.04)',
  },
  {
    id: 'monochrome',
    name: 'Monochrome',
    description: 'Neutral greys — pure focus',
    accent: '#A1A1A6',
    accentLight: '#D4D4D8',
    accentDim: 'rgba(161,161,166,0.15)',
    accentGlow: 'rgba(161,161,166,0.3)',
    secondary: '#71717A',
    secondaryDim: 'rgba(113,113,122,0.15)',
    selection: '#A1A1A6',
    selectionDim: 'rgba(161,161,166,0.12)',
    glowRed: '0 0 40px rgba(161,161,166,0.08), 0 0 80px rgba(161,161,166,0.04)',
  },
]

function loadCustomPalettes(): ThemePalette[] {
  try {
    const raw = localStorage.getItem(CUSTOM_KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed.filter(isValidPalette).map(p => ({ ...p, custom: true }))
  } catch {
    return []
  }
}

function saveCustomPalettes(customs: ThemePalette[]) {
  try { localStorage.setItem(CUSTOM_KEY, JSON.stringify(customs)) } catch { /* ignore */ }
}

const PALETTE_COLOR_KEYS = [
  'accent', 'accentLight', 'accentDim', 'accentGlow',
  'secondary', 'secondaryDim', 'selection', 'selectionDim', 'glowRed',
] as const

export function isValidPalette(v: any): v is ThemePalette {
  if (!v || typeof v !== 'object') return false
  if (typeof v.id !== 'string' || !v.id) return false
  if (typeof v.name !== 'string' || !v.name) return false
  for (const k of PALETTE_COLOR_KEYS) {
    if (typeof v[k] !== 'string') return false
  }
  return true
}

export function allPalettes(): ThemePalette[] {
  return [...THEME_PRESETS, ...loadCustomPalettes()]
}

export function getActiveThemeId(): string {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (raw && allPalettes().some(p => p.id === raw)) return raw
  } catch { /* ignore */ }
  return 'hardwaveRed'
}

export function getActiveTheme(): ThemePalette {
  const id = getActiveThemeId()
  return allPalettes().find(p => p.id === id) ?? THEME_PRESETS[0]
}

export function derivePaletteFromAccent(
  id: string, name: string, description: string, accent: string, secondary: string,
): ThemePalette {
  const toRgba = (hex: string, alpha: number) => {
    const m = hex.replace('#', '')
    const r = parseInt(m.length === 3 ? m[0] + m[0] : m.slice(0, 2), 16)
    const g = parseInt(m.length === 3 ? m[1] + m[1] : m.slice(2, 4), 16)
    const b = parseInt(m.length === 3 ? m[2] + m[2] : m.slice(4, 6), 16)
    return `rgba(${r},${g},${b},${alpha})`
  }
  const lighten = (hex: string) => {
    const m = hex.replace('#', '')
    const r = Math.min(255, parseInt(m.slice(0, 2), 16) + 30)
    const g = Math.min(255, parseInt(m.slice(2, 4), 16) + 30)
    const b = Math.min(255, parseInt(m.slice(4, 6), 16) + 30)
    return `#${[r, g, b].map(v => v.toString(16).padStart(2, '0')).join('').toUpperCase()}`
  }
  return {
    id, name, description,
    accent,
    accentLight: lighten(accent),
    accentDim: toRgba(accent, 0.15),
    accentGlow: toRgba(accent, 0.3),
    secondary,
    secondaryDim: toRgba(secondary, 0.15),
    selection: accent,
    selectionDim: toRgba(accent, 0.12),
    glowRed: `0 0 40px ${toRgba(accent, 0.08)}, 0 0 80px ${toRgba(accent, 0.04)}`,
    custom: true,
  }
}

function loadCustomBg(): string {
  try { return localStorage.getItem(CUSTOM_BG_KEY) ?? '' } catch { return '' }
}

function saveCustomBg(bg: string) {
  try {
    if (bg) localStorage.setItem(CUSTOM_BG_KEY, bg)
    else localStorage.removeItem(CUSTOM_BG_KEY)
  } catch { /* ignore */ }
}

export function applyCustomBg(bg: string) {
  if (typeof document === 'undefined') return
  if (bg) document.body.style.background = bg
  else document.body.style.removeProperty('background')
}

interface ThemeState {
  activeId: string
  customs: ThemePalette[]
  customBg: string
  setTheme: (id: string) => void
  setCustomBg: (bg: string) => void
  addCustom: (palette: ThemePalette) => boolean
  removeCustom: (id: string) => void
  reloadCustoms: () => void
}

export const useThemeStore = create<ThemeState>((set, get) => ({
  activeId: getActiveThemeId(),
  customs: loadCustomPalettes(),
  customBg: loadCustomBg(),
  setCustomBg: (bg) => {
    const trimmed = bg.trim()
    saveCustomBg(trimmed)
    applyCustomBg(trimmed)
    set({ customBg: trimmed })
  },
  setTheme: (id) => {
    if (!allPalettes().some(p => p.id === id)) return
    try { localStorage.setItem(STORAGE_KEY, id) } catch { /* ignore */ }
    set({ activeId: id })
  },
  addCustom: (palette) => {
    if (!isValidPalette(palette)) return false
    if (THEME_PRESETS.some(p => p.id === palette.id)) return false
    const customs = get().customs.filter(c => c.id !== palette.id)
    const next = [...customs, { ...palette, custom: true }]
    saveCustomPalettes(next)
    set({ customs: next })
    return true
  },
  removeCustom: (id) => {
    const next = get().customs.filter(c => c.id !== id)
    saveCustomPalettes(next)
    const activeId = get().activeId === id ? 'hardwaveRed' : get().activeId
    if (activeId !== get().activeId) {
      try { localStorage.setItem(STORAGE_KEY, activeId) } catch { /* ignore */ }
    }
    set({ customs: next, activeId })
  },
  reloadCustoms: () => set({ customs: loadCustomPalettes() }),
}))
