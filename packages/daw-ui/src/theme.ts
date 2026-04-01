/** FL Studio 21 default theme — warm medium-dark gray, orange accents, green LEDs */
export const hw = {
  // Backgrounds — FL's warm medium-dark grays (NOT near-black)
  bg: '#464646',           // main panel background
  bgDark: '#3E3E3E',       // slightly darker panels
  bgDeep: '#333333',       // deepest (title bar, panel headers)
  bgToolbar: '#525252',    // toolbar base
  bgToolbarGrad: 'linear-gradient(180deg, #606060 0%, #525252 40%, #484848 100%)',
  bgPanel: '#444444',      // channel rack / mixer background
  bgCard: '#4E4E4E',       // raised card elements
  bgHover: '#555555',      // hover highlight
  bgInput: '#2A2A2A',      // LCD/input recessed areas

  // Text — FL's light gray text
  textPrimary: '#DDDDDD',
  textSecondary: '#C0C0C0',
  textMuted: '#999999',
  textFaint: '#777777',
  textBright: '#FFFFFF',

  // Borders — FL style
  border: 'rgba(255, 255, 255, 0.1)',
  borderLight: 'rgba(255, 255, 255, 0.15)',
  borderDark: 'rgba(0, 0, 0, 0.4)',

  // FL Orange — the signature channel rack color
  orange: '#E85D00',
  orangeLight: '#FF7722',
  orangeDim: 'rgba(232, 93, 0, 0.2)',

  // FL Green — LEDs and LCD digits
  green: '#00CC44',
  greenBright: '#00FF44',
  greenDim: 'rgba(0, 204, 68, 0.15)',
  greenLcd: '#00DD44',    // LCD digit color

  // Red — record, mute
  red: '#CC3333',
  redDim: 'rgba(204, 51, 51, 0.15)',

  // Yellow — solo, warning
  yellow: '#DDAA00',
  yellowDim: 'rgba(221, 170, 0, 0.15)',

  blue: '#5599DD',

  // Selection / active highlight
  selection: '#5588BB',
  selectionDim: 'rgba(85, 136, 187, 0.2)',

  // Clip colors — FL Studio's pattern/clip palette
  clips: [
    '#C06060', '#60A060', '#6060C0', '#C0A060',
    '#60C0C0', '#C060C0', '#A0C060', '#6080C0',
  ],

  // Radius — FL uses minimal rounding
  radius: { sm: 1, md: 2, lg: 4 },
} as const
