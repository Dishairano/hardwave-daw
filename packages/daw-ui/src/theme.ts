/** Hardwave DAW design tokens — purple atmospheric theme inspired by FL Studio VEEL */
export const hw = {
  // Backgrounds — charcoal with subtle warmth
  bg: '#1E1E22',
  bgDark: '#18181C',
  bgDeep: '#141418',
  bgToolbar: '#2C2C30',
  bgToolbarGrad: 'linear-gradient(180deg, #353538 0%, #2C2C30 50%, #252528 100%)',
  bgPanel: '#222226',
  bgCard: '#2A2A2E',
  bgHover: '#333338',
  bgInput: '#1A1A1E',

  // Text
  textPrimary: '#E8E8EC',
  textSecondary: '#B0B0B8',
  textMuted: '#808088',
  textFaint: '#58585F',
  textBright: '#FFFFFF',

  // Borders
  border: 'rgba(255, 255, 255, 0.07)',
  borderLight: 'rgba(255, 255, 255, 0.12)',
  borderDark: 'rgba(0, 0, 0, 0.3)',

  // Brand purple — the atmospheric accent
  purple: '#9B6DFF',
  purpleLight: '#B48EFF',
  purpleDim: 'rgba(155, 109, 255, 0.15)',
  purpleGlow: 'rgba(155, 109, 255, 0.3)',
  purpleMuted: '#7B5AC0',

  // Hardwave red (secondary accent)
  red: '#EF4444',
  redDim: 'rgba(239, 68, 68, 0.15)',

  // Functional
  green: '#4ADE80',
  greenDim: 'rgba(74, 222, 128, 0.15)',
  yellow: '#FBBF24',
  yellowDim: 'rgba(251, 191, 36, 0.15)',
  blue: '#60A5FA',

  // Clip colors — vibrant on dark background
  clips: [
    '#7C6AE8', '#5B9FE8', '#E86A8A', '#4AD4A0',
    '#E8A04A', '#D46AE8', '#4AE8D0', '#E86A4A',
  ],

  // Radius
  radius: { sm: 3, md: 5, lg: 8 },
} as const
