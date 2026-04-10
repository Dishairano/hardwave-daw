/** Hardwave DAW — Dark purple-accent theme with near-black backgrounds */
export const hw = {
  // ─── Backgrounds ─────────────────────────────────────────────────────────────
  bg: '#0c0c10',              // root / deepest background
  bgPanel: '#111116',         // side panels, browser, track list
  bgSurface: '#17171e',      // cards, elevated panels
  bgElevated: '#1e1e28',     // hover, raised elements
  bgInput: '#08080c',        // recessed inputs, LCD areas
  bgToolbar: '#131318',      // toolbar base
  bgToolbarGrad: 'linear-gradient(180deg, #1a1a22 0%, #141419 40%, #101015 100%)',

  // ─── Text ────────────────────────────────────────────────────────────────────
  textPrimary: '#e0e0ec',
  textSecondary: '#9898b0',
  textMuted: '#606078',
  textFaint: '#3c3c50',
  textBright: '#ffffff',

  // ─── Borders ─────────────────────────────────────────────────────────────────
  border: 'rgba(255, 255, 255, 0.06)',
  borderLight: 'rgba(255, 255, 255, 0.10)',
  borderDark: 'rgba(0, 0, 0, 0.6)',

  // ─── Accent — Hardwave Purple ────────────────────────────────────────────────
  accent: '#9B6DFF',
  accentLight: '#B48EFF',
  accentDim: 'rgba(155, 109, 255, 0.15)',
  accentGlow: 'rgba(155, 109, 255, 0.3)',

  // ─── Secondary — Deep Purple ─────────────────────────────────────────────────
  secondary: '#7B5AC0',
  secondaryDim: 'rgba(123, 90, 192, 0.15)',

  // ─── Meters / Success — Teal ─────────────────────────────────────────────────
  green: '#00D4AA',
  greenBright: '#00FFCC',
  greenDim: 'rgba(0, 212, 170, 0.15)',
  greenLcd: '#C4B5FF',       // LCD digit color — purple-tinted

  // ─── Record / Danger — Pink-Red ──────────────────────────────────────────────
  red: '#FF4466',
  redDim: 'rgba(255, 68, 102, 0.15)',

  // ─── Solo / Warning — Amber ──────────────────────────────────────────────────
  yellow: '#FFB020',
  yellowDim: 'rgba(255, 176, 32, 0.15)',

  // ─── Info — Blue ─────────────────────────────────────────────────────────────
  blue: '#4488FF',

  // ─── Selection ───────────────────────────────────────────────────────────────
  selection: '#9B6DFF',
  selectionDim: 'rgba(155, 109, 255, 0.12)',

  // ─── Playhead ────────────────────────────────────────────────────────────────
  orange: '#9B6DFF',          // legacy name kept for compat — now purple
  orangeLight: '#B48EFF',
  orangeDim: 'rgba(155, 109, 255, 0.2)',

  // ─── Clip Colors — Vibrant on dark ───────────────────────────────────────────
  clips: [
    '#9B6DFF', '#00D4AA', '#FF4466', '#FFB020',
    '#4488FF', '#FF66AA', '#66DDFF', '#88EE66',
  ],

  // ─── Radius ──────────────────────────────────────────────────────────────────
  radius: { sm: 3, md: 6, lg: 10 },
} as const
