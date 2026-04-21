import { create } from 'zustand'

export const STORAGE_KEY = 'hardwave.daw.theme'

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

export function getActiveThemeId(): string {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (raw && THEME_PRESETS.some(p => p.id === raw)) return raw
  } catch { /* ignore */ }
  return 'hardwaveRed'
}

export function getActiveTheme(): ThemePalette {
  const id = getActiveThemeId()
  return THEME_PRESETS.find(p => p.id === id) ?? THEME_PRESETS[0]
}

interface ThemeState {
  activeId: string
  setTheme: (id: string) => void
}

export const useThemeStore = create<ThemeState>((set) => ({
  activeId: getActiveThemeId(),
  setTheme: (id) => {
    if (!THEME_PRESETS.some(p => p.id === id)) return
    try { localStorage.setItem(STORAGE_KEY, id) } catch { /* ignore */ }
    set({ activeId: id })
  },
}))
