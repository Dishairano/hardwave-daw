/** Hardwave DAW — Suite-matched dark glassmorphic theme with red accents */
export const hw = {
  // ─── Backgrounds ─────────────────────────────────────────────────────────────
  bg: '#08080c',              // root / deepest background (Suite base)
  bgPanel: 'rgba(255,255,255,0.02)',  // glass panels
  bgSurface: 'rgba(255,255,255,0.03)', // elevated glass
  bgElevated: 'rgba(255,255,255,0.06)', // hover / raised
  bgInput: 'rgba(255,255,255,0.04)',    // recessed inputs
  bgToolbar: 'rgba(255,255,255,0.02)',
  bgToolbarGrad: 'linear-gradient(180deg, rgba(255,255,255,0.04) 0%, rgba(255,255,255,0.02) 40%, rgba(255,255,255,0.01) 100%)',
  // Solid fallbacks for canvas (canvas can't use rgba well)
  bgCanvasDark: '#0a0a0f',
  bgCanvasRow1: '#0b0b10',
  bgCanvasRow2: '#0d0d12',

  // ─── Text ────────────────────────────────────────────────────────────────────
  textPrimary: '#fafafa',     // zinc-50
  textSecondary: '#d4d4d8',   // zinc-300
  textMuted: '#a1a1a6',       // zinc-400
  textFaint: '#52525b',       // zinc-600
  textBright: '#ffffff',

  // ─── Borders (glass style) ───────────────────────────────────────────────────
  border: 'rgba(255,255,255,0.06)',
  borderLight: 'rgba(255,255,255,0.08)',
  borderDark: 'rgba(255,255,255,0.04)',

  // ─── Accent — Hardwave Red (matches Suite) ──────────────────────────────────
  accent: '#DC2626',          // red-600
  accentLight: '#EF4444',     // red-500
  accentDim: 'rgba(220,38,38,0.15)',
  accentGlow: 'rgba(220,38,38,0.3)',

  // ─── Secondary — Red-700 ────────────────────────────────────────────────────
  secondary: '#B91C1C',       // red-700
  secondaryDim: 'rgba(185,28,28,0.15)',

  // ─── Meters / Success — Emerald ──────────────────────────────────────────────
  green: '#10B981',           // emerald-500
  greenBright: '#34D399',     // emerald-400
  greenDim: 'rgba(16,185,129,0.15)',
  greenLcd: '#fafafa',        // LCD digits — white (Suite style)

  // ─── Record / Danger — Red ───────────────────────────────────────────────────
  red: '#EF4444',             // red-500
  redDim: 'rgba(239,68,68,0.12)',

  // ─── Solo / Warning — Amber ──────────────────────────────────────────────────
  yellow: '#F59E0B',          // amber-500
  yellowDim: 'rgba(245,158,11,0.15)',

  // ─── Info — Violet ───────────────────────────────────────────────────────────
  blue: '#7C3AED',            // violet-600

  // ─── Selection ───────────────────────────────────────────────────────────────
  selection: '#DC2626',
  selectionDim: 'rgba(220,38,38,0.12)',

  // ─── Playhead ────────────────────────────────────────────────────────────────
  orange: '#DC2626',          // legacy alias — now red
  orangeLight: '#EF4444',
  orangeDim: 'rgba(220,38,38,0.2)',

  // ─── Clip Colors — Vibrant on dark ───────────────────────────────────────────
  clips: [
    '#DC2626', '#10B981', '#A855F7', '#F59E0B',
    '#3B82F6', '#EC4899', '#06B6D4', '#84CC16',
  ],

  // ─── Radius (Suite uses larger radii) ────────────────────────────────────────
  radius: { sm: 6, md: 8, lg: 12 },

  // ─── Backdrop blur ───────────────────────────────────────────────────────────
  blur: {
    sm: 'blur(4px)',
    md: 'blur(8px)',
  },

  // ─── Glow ────────────────────────────────────────────────────────────────────
  glowRed: '0 0 40px rgba(220,38,38,0.08), 0 0 80px rgba(220,38,38,0.04)',
} as const
