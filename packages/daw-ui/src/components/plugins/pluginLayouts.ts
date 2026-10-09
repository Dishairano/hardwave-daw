// The windows of the built-in plug-ins, in the lineup's design (WettBoi,
// WideBoi, KickForge): a family colour, a header with presets, A/B, mix and
// bypass, and per plug-in a set of panels. Approved as the mockup at
// suite.hardwavestudios.com/daw-plugin-ui-mockup/ (2026-10-08).
//
// Everything a window shows is either a parameter of the plug-in or live
// data from the engine (the slot's levels and gain reduction). Displays
// draw what the parameters say; none of them pretends to be a meter.

export type Family = 'dyn' | 'eq' | 'time' | 'mod' | 'drive' | 'space' | 'synth' | 'fx'

export interface FamilyColours { a: string; a2: string; deep: string }

export const FAMILY: Record<Family, FamilyColours> = {
  dyn:   { a: '#F59E0B', a2: '#FBBF24', deep: '#92400E' },
  eq:    { a: '#14B8A6', a2: '#2DD4BF', deep: '#115E59' },
  time:  { a: '#6366F1', a2: '#818CF8', deep: '#3730A3' },
  mod:   { a: '#8B5CF6', a2: '#A78BFA', deep: '#5B21B6' },
  drive: { a: '#DC2626', a2: '#EF4444', deep: '#7F1D1D' },
  space: { a: '#0EA5E9', a2: '#38BDF8', deep: '#0369A1' },
  synth: { a: '#EC4899', a2: '#F472B6', deep: '#9D174D' },
  fx:    { a: '#84CC16', a2: '#A3E635', deep: '#3F6212' },
}

export const FAMILY_NAME: Record<Family, string> = {
  dyn: 'Dynamics', eq: 'EQ', time: 'Time', mod: 'Modulation',
  drive: 'Drive', space: 'Space', synth: 'Synth', fx: 'FX',
}

/** One row inside a panel section. */
export type Row =
  | ['k', ...string[]]          // knobs
  | ['K', string]               // one big knob
  | ['p', string]               // choices as pills
  | ['w', string]               // an on/off switch row
  | ['d', string, number]       // a display of this kind, this tall
  | ['r', ...Readout[]]         // read-outs
  | ['n', string]               // a note
  | ['m']                       // the slot's IN and OUT levels
  | ['g']                       // the slot's gain reduction
  | ['load']                    // the sampler's "Load sample" button

/**
 * A read-out. Each is measured (the slot's levels and gain reduction) or
 * worked out from the plug-in's own parameters; none is a stand-in.
 */
export type Readout =
  | 'in' | 'out'                // peak into and out of the slot
  | 'gr' | 'grmax'              // gain reduction now, deepest in 4 s
  | 'gate'                      // open or closed, from the reduction
  | 'corr'                      // stereo correlation of the output
  | 'repeats' | 'tail'          // a delay's audible repeats and how long
  | `param:${string}`           // a parameter's own text

export interface Panel {
  t: string
  /** A switch parameter shown in the header instead of the plain ON. */
  on?: string
  x?: string
  grow?: boolean
  s: Row[][]
}

export interface PluginLayout {
  sub: string
  fam: Family
  w: number
  /** CSS grid columns; one entry per column of panels. */
  cols?: string
  panels?: Panel[][]
  top?: Panel
  /** A wide panel under the columns; its rows sit side by side. */
  bottom?: Panel
  /** A parameter shown as the header's Mix. */
  mix?: string
  /** A choice shown as a segmented control in the header. */
  hp?: string
  /** Instruments have no input, so no IN strip. */
  outOnly?: boolean
  custom?: 'eq'
}

const levels: Row = ['m']

// The approved mockup, with every stand-in swapped for something real: the
// meters and read-outs are the slot's own levels and gain reduction, the
// stereo fields draw the slot's output, and the displays draw what the
// parameters set. Footers carry no invented facts.
export const LAYOUTS: Record<string, PluginLayout> = {
  eq: { sub: 'Parametric equaliser', fam: 'eq', w: 1000, custom: 'eq' },
  compressor: { sub: 'Dynamics · Compressor', fam: 'dyn', w: 1000, cols: '1fr 1.25fr 0.8fr', panels: [
    [{ t: 'Detector', s: [[['k', 'Threshold', 'Ratio', 'Knee']], [['p', 'Detect Mode']]] },
     { t: 'Envelope', grow: true, s: [[['k', 'Attack', 'Release']], [['n', 'Attack is how fast the level comes down once it passes the threshold, release how fast it comes back.']]] }],
    [{ t: 'Transfer', x: 'IN → OUT', grow: true, s: [[['d', 'transfer', 236]], [['r', 'in', 'out', 'gr']]] }],
    [{ t: 'Output', grow: true, s: [[['K', 'Makeup']], [['w', 'Auto Makeup']], [['g']], [levels]] }],
  ] },
  limiter: { sub: 'Dynamics · Brickwall limiter', fam: 'dyn', w: 920, cols: '1fr 1.2fr', panels: [
    [{ t: 'Limiter', s: [[['K', 'Drive']], [['k', 'Threshold', 'Ceiling', 'Release']]] },
     { t: 'Output', grow: true, s: [[levels], [['n', 'The ceiling is the highest the output can go. Nothing passes it.']]] }],
    [{ t: 'Gain reduction', x: 'last 4 s', grow: true, s: [[['d', 'grhistory', 240]], [['r', 'in', 'out', 'grmax']]] }],
  ] },
  gate: { sub: 'Dynamics · Gate', fam: 'dyn', w: 920, cols: '1fr 1.2fr', panels: [
    [{ t: 'Gate', s: [[['k', 'Threshold', 'Range', 'Hysteresis']]] }, { t: 'Envelope', grow: true, s: [[['k', 'Attack', 'Release']], [levels]] }],
    [{ t: 'Activity', x: 'open / closed', grow: true, s: [[['d', 'gate', 250]], [['r', 'gate', 'gr']]] }],
  ] },
  transient: { sub: 'Dynamics · Transient shaper', fam: 'dyn', w: 860, cols: '1.2fr 1fr', panels: [
    [{ t: 'Shape', grow: true, s: [[['d', 'transient', 230]], [['k', 'Attack', 'Sustain']]] }],
    [{ t: 'Output', grow: true, s: [[['K', 'Output']], [levels], [['n', 'Attack lifts or softens the start of each hit, sustain the body after it. It does not depend on level.']]] }],
  ] },
  clipper: { sub: 'Dynamics · Clipper', fam: 'dyn', w: 960, cols: '1fr 1.2fr', hp: 'Oversample', panels: [
    [{ t: 'Clipper', s: [[['K', 'Drive']], [['k', 'Ceiling']], [['w', 'Auto-Gain']]] }, { t: 'Output', grow: true, s: [[levels]] }],
    [{ t: 'Curve', grow: true, s: [[['d', 'clip', 260]], [['r', 'in', 'out']]] }],
  ] },
  multiband: { sub: 'Dynamics · Multiband compressor', fam: 'dyn', w: 1080, cols: '1fr 1fr 1fr 0.8fr', panels: [
    [{ t: 'Low', grow: true, s: [[['k', 'Low Thresh', 'Low Ratio']]] }],
    [{ t: 'Mid', grow: true, s: [[['k', 'Mid Thresh', 'Mid Ratio']]] }],
    [{ t: 'High', grow: true, s: [[['k', 'High Thresh', 'High Ratio']]] }],
    [{ t: 'Output', grow: true, s: [[['K', 'Output']], [['g']], [levels]] }],
  ], top: { t: 'Bands', x: 'drag the crossovers', s: [[['d', 'xover', 150]], [['k', 'Low/Mid', 'Mid/High']]] } },
  filter: { sub: 'EQ · Filter', fam: 'eq', w: 920, cols: '1.4fr 1fr', mix: 'Mix', panels: [
    [{ t: 'Response', grow: true, s: [[['d', 'filter', 250]], [['p', 'Mode']]] }],
    [{ t: 'Filter', grow: true, s: [[['K', 'Cutoff']], [['k', 'Q']], [levels]] }],
  ] },
  auto_filter: { sub: 'EQ · Envelope filter', fam: 'eq', w: 980, cols: '1.3fr 1fr', panels: [
    [{ t: 'Response', x: 'sweep range', grow: true, s: [[['d', 'filter', 230]], [['k', 'Base', 'Range', 'Resonance']]] }],
    [{ t: 'Follower', s: [[['K', 'Sense']], [['k', 'Attack', 'Release']]] }, { t: 'Envelope', grow: true, s: [[['d', 'follow', 90]], [levels]] }],
  ] },
  delay: { sub: 'Time · Delay', fam: 'time', w: 980, cols: '1fr 1.3fr', mix: 'Mix', panels: [
    [{ t: 'Delay', s: [[['p', 'Ping-Pong']], [['K', 'Time']], [['k', 'Feedback']]] }, { t: 'Output', grow: true, s: [[levels]] }],
    [{ t: 'Echoes', x: 'feedback decay', grow: true, s: [[['d', 'taps', 250]], [['r', 'param:Time', 'repeats', 'tail']]] }],
  ] },
  reverb: { sub: 'Time · Reverb', fam: 'time', w: 1000, cols: '1fr 1.25fr', mix: 'Mix', panels: [
    [{ t: 'Space', s: [[['k', 'Size', 'Decay']], [['k', 'Damping', 'Pre-Delay']]] }, { t: 'Output', grow: true, s: [[levels]] }],
    [{ t: 'Decay', x: 'impulse', grow: true, s: [[['d', 'decay', 260]], [['r', 'param:Decay', 'param:Pre-Delay', 'param:Size']]] }],
  ] },
  conv_reverb: { sub: 'Time · Convolution reverb', fam: 'time', w: 1040, cols: '1fr 1.25fr', mix: 'Mix', panels: [
    [{ t: 'Impulse', s: [[['p', 'Preset']], [['k', 'Pre-Delay', 'Width', 'Tail Length']]] }, { t: 'Tone', grow: true, s: [[['k', 'Low Cut', 'High Cut']]] }],
    [{ t: 'Impulse response', x: 'kept part', grow: true, s: [[['d', 'ir', 260]], [levels]] }],
  ] },
  chorus: { sub: 'Modulation · Chorus', fam: 'mod', w: 940, cols: '1fr 1.2fr', mix: 'Mix', panels: [
    [{ t: 'Chorus', s: [[['k', 'Rate', 'Depth', 'Feedback']]] }, { t: 'Stereo', grow: true, s: [[['K', 'Width']]] }],
    [{ t: 'LFO', x: 'L / R over 1 s', grow: true, s: [[['d', 'lfo2', 230]], [levels]] }],
  ] },
  phaser: { sub: 'Modulation · Phaser', fam: 'mod', w: 980, cols: '1fr 1.25fr', mix: 'Mix', panels: [
    [{ t: 'Phaser', s: [[['k', 'Rate', 'Depth']], [['k', 'Base', 'Spread']]] }, { t: 'Output', grow: true, s: [[levels]] }],
    [{ t: 'Notches', x: 'sweep range', grow: true, s: [[['d', 'notches', 260]]] }],
  ] },
  flanger: { sub: 'Modulation · Flanger', fam: 'mod', w: 920, cols: '1fr 1.2fr', mix: 'Mix', panels: [
    [{ t: 'Flanger', s: [[['k', 'Rate', 'Depth', 'Feedback']], [['w', 'Invert']]] }, { t: 'Output', grow: true, s: [[levels]] }],
    [{ t: 'Comb', x: 'at full depth', grow: true, s: [[['d', 'comb', 250]]] }],
  ] },
  tremolo: { sub: 'Modulation · Tremolo', fam: 'mod', w: 900, cols: '1fr 1.2fr', panels: [
    [{ t: 'Tremolo', s: [[['p', 'Shape']], [['k', 'Rate', 'Depth', 'Stereo']]] }, { t: 'Output', grow: true, s: [[levels]] }],
    [{ t: 'LFO', x: 'level over 1 s', grow: true, s: [[['d', 'lfo', 240]]] }],
  ] },
  auto_pan: { sub: 'Modulation · Auto-pan', fam: 'mod', w: 880, cols: '1fr 1.2fr', panels: [
    [{ t: 'Auto-pan', grow: true, s: [[['p', 'Shape']], [['k', 'Rate', 'Depth']], [levels]] }],
    [{ t: 'Position', x: 'L ↕ R over 1 s', grow: true, s: [[['d', 'pan', 240]]] }],
  ] },
  vibrato: { sub: 'Modulation · Vibrato', fam: 'mod', w: 820, cols: '1fr 1.2fr', panels: [
    [{ t: 'Vibrato', grow: true, s: [[['K', 'Rate']], [['k', 'Depth']]] }],
    [{ t: 'Pitch', x: 'over 1 s', grow: true, s: [[['d', 'lfo', 220]], [levels]] }],
  ] },
  ring_mod: { sub: 'Modulation · Ring modulator', fam: 'mod', w: 860, cols: '1fr 1.2fr', mix: 'Mix', panels: [
    [{ t: 'Carrier', grow: true, s: [[['K', 'Frequency']], [levels]] }],
    [{ t: 'Spectrum', x: 'a 440 Hz tone in', grow: true, s: [[['d', 'ring', 240]]] }],
  ] },
  distortion: { sub: 'Drive · Distortion', fam: 'drive', w: 1000, cols: '1fr 1.2fr', mix: 'Mix', hp: 'Oversample', panels: [
    [{ t: 'Drive', s: [[['p', 'Mode']], [['K', 'Drive']]] }, { t: 'Output', grow: true, s: [[['k', 'Output']], [levels]] }],
    [{ t: 'Transfer', x: 'waveshaper', grow: true, s: [[['d', 'shaper', 280]], [['r', 'in', 'out']]] }],
  ] },
  saturator: { sub: 'Drive · Saturator', fam: 'drive', w: 960, cols: '1fr 1.2fr', mix: 'Mix', hp: 'Oversample', panels: [
    [{ t: 'Saturation', s: [[['K', 'Amount']]] }, { t: 'Output', grow: true, s: [[['k', 'Output']], [levels]] }],
    [{ t: 'Transfer', x: 'waveshaper', grow: true, s: [[['d', 'shaper', 270]]] }],
  ] },
  tape: { sub: 'Drive · Tape', fam: 'drive', w: 940, cols: '1fr 1.2fr', panels: [
    [{ t: 'Machine', s: [[['K', 'Drive']], [['k', 'Wow', 'HF Loss']]] }, { t: 'Output', grow: true, s: [[['k', 'Output']], [levels]] }],
    [{ t: 'Reels', x: 'turn while sound passes', grow: true, s: [[['d', 'reels', 280]]] }],
  ] },
  bitcrush: { sub: 'Drive · Bitcrusher', fam: 'drive', w: 900, cols: '1fr 1.2fr', mix: 'Mix', panels: [
    [{ t: 'Crush', s: [[['k', 'Bits', 'Rate']]] }, { t: 'Drive', grow: true, s: [[['K', 'Drive']], [levels]] }],
    [{ t: 'Waveform', x: 'a sine through it', grow: true, s: [[['d', 'steps', 270]]] }],
  ] },
  exciter: { sub: 'Drive · Exciter', fam: 'drive', w: 880, cols: '1fr 1.2fr', mix: 'Mix', panels: [
    [{ t: 'Exciter', grow: true, s: [[['K', 'Amount']], [['k', 'Frequency']], [levels]] }],
    [{ t: 'Added harmonics', x: 'above the frequency', grow: true, s: [[['d', 'excite', 260]]] }],
  ] },
  sub_bass: { sub: 'Drive · Sub bass', fam: 'drive', w: 900, cols: '1fr 1.2fr', mix: 'Mix', panels: [
    [{ t: 'Sub', s: [[['k', 'Crossover', 'Boost']]] }, { t: 'Drive', grow: true, s: [[['K', 'Drive']], [levels]] }],
    [{ t: 'Low end', x: 'below the crossover', grow: true, s: [[['d', 'sub', 260]]] }],
  ] },
  soundgoodizer: { sub: 'Drive · Master', fam: 'drive', w: 760, cols: '1fr 1fr', panels: [
    [{ t: 'Amount', grow: true, s: [[['K', 'Amount']], [['k', 'Output']]] }],
    [{ t: 'Loudness', x: 'in / out', grow: true, s: [[['d', 'lr', 200]], [levels]] }],
  ] },
  stereo: { sub: 'Space · Stereo', fam: 'space', w: 980, cols: '1.2fr 1fr', panels: [
    [{ t: 'Stereo field', x: 'goniometer', grow: true, s: [[['d', 'gonio', 270]], [['r', 'corr', 'param:Width']]] }],
    [{ t: 'Width', s: [[['K', 'Width']], [['k', 'Balance']]] }, { t: 'Low end', on: 'Bass Mono', grow: true, s: [[['k', 'Crossover']], [['n', 'Everything under the crossover is summed to mono, so the kick and sub stay centred however wide the rest goes.']]] }],
  ] },
  mid_side: { sub: 'Space · Mid / side', fam: 'space', w: 880, cols: '1fr 1.2fr', panels: [
    [{ t: 'Balance', s: [[['k', 'Mid', 'Side']], [['p', 'Solo']]] }, { t: 'Output', grow: true, s: [[levels]] }],
    [{ t: 'Stereo field', x: 'goniometer', grow: true, s: [[['d', 'gonio', 260]], [['r', 'corr']]] }],
  ] },
  stereo_double: { sub: 'Space · Doubler', fam: 'space', w: 940, cols: '1fr 1.2fr', mix: 'Mix', panels: [
    [{ t: 'Doubler', s: [[['k', 'Delay', 'Detune']]] }, { t: 'Spread', grow: true, s: [[['K', 'Spread']]] }],
    [{ t: 'Stereo field', x: 'goniometer', grow: true, s: [[['d', 'gonio', 250]], [levels]] }],
  ] },
  mono_fold: { sub: 'Space · Mono', fam: 'space', w: 720, cols: '1fr 1fr', panels: [
    [{ t: 'Fold', grow: true, s: [[['K', 'Amount']]] }],
    [{ t: 'Stereo field', grow: true, s: [[['d', 'gonio', 200]], [levels]] }],
  ] },
  gain: { sub: 'Space · Gain', fam: 'space', w: 780, cols: '1fr 1fr', panels: [
    [{ t: 'Gain', s: [[['K', 'Gain']], [['k', 'Pan']]] }, { t: 'Polarity', grow: true, s: [[['w', 'Invert L']], [['w', 'Invert R']], [['w', 'Mute']]] }],
    [{ t: 'Level', x: 'out', grow: true, s: [[['d', 'lr', 220]], [['r', 'in', 'out']]] }],
  ] },
  tripleosc: { sub: 'Synth · 3 oscillators', fam: 'synth', w: 1080, cols: '1fr 1fr 1fr', outOnly: true, panels: [
    [{ t: 'Oscillator 1', s: [[['d', 'wave:Osc1 Wave', 70]], [['p', 'Osc1 Wave']], [['k', 'Osc1 Detune', 'Osc1 Level']]] }],
    [{ t: 'Oscillator 2', s: [[['d', 'wave:Osc2 Wave', 70]], [['p', 'Osc2 Wave']], [['k', 'Osc2 Detune', 'Osc2 Level']]] }],
    [{ t: 'Oscillator 3', s: [[['d', 'wave:Osc3 Wave', 70]], [['p', 'Osc3 Wave']], [['k', 'Osc3 Detune', 'Osc3 Level']]] }],
  ], bottom: { t: 'Amp envelope', x: 'ADSR', s: [[['d', 'adsr', 150], ['k', 'Attack', 'Decay', 'Sustain', 'Release']]] } },
  fm: { sub: 'Synth · 4-operator FM', fam: 'synth', w: 1080, cols: '1fr 1.6fr', outOnly: true, panels: [
    [{ t: 'Algorithm', grow: true, s: [[['d', 'algo', 170]], [['p', 'Algorithm']]] }],
    [{ t: 'Operators', grow: true, s: [[['k', 'Op1 Ratio', 'Op2 Ratio', 'Op3 Ratio', 'Op4 Ratio']], [['k', 'Op1 Level', 'Op2 Level', 'Op3 Level', 'Op4 Level']]] }],
  ], bottom: { t: 'Amp envelope', x: 'ADSR', s: [[['d', 'adsr', 130], ['k', 'Attack', 'Decay', 'Sustain', 'Release', 'Master']]] } },
  wavetable: { sub: 'Synth · Wavetable', fam: 'synth', w: 1040, cols: '1.4fr 1fr', outOnly: true, panels: [
    [{ t: 'Wavetable', grow: true, s: [[['d', 'table', 220]], [['p', 'Bank']]] }],
    [{ t: 'Oscillator', s: [[['K', 'Position']], [['k', 'Detune', 'Master']]] }, { t: 'Amp envelope', grow: true, s: [[['d', 'adsr', 80]], [['k', 'Attack', 'Decay', 'Sustain', 'Release']]] }],
  ] },
  noise: { sub: 'Synth · Noise', fam: 'synth', w: 920, cols: '1fr 1.2fr', outOnly: true, panels: [
    [{ t: 'Noise', s: [[['p', 'Colour']], [['K', 'Level']]] }, { t: 'Amp envelope', grow: true, s: [[['k', 'Attack', 'Decay', 'Sustain', 'Release']]] }],
    [{ t: 'Spectrum', x: 'slope', grow: true, s: [[['d', 'noise', 170]], [['d', 'adsr', 90]]] }],
  ] },
  sampler: { sub: 'Synth · Sampler', fam: 'synth', w: 900, cols: '1.4fr 1fr', outOnly: true, panels: [
    [{ t: 'Sample', grow: true, s: [[['d', 'ar', 200]], [['load']]] }],
    [{ t: 'Amp', grow: true, s: [[['K', 'Gain']], [['k', 'Attack', 'Release']]] }],
  ] },
  vocoder: { sub: 'FX · Vocoder', fam: 'fx', w: 1000, cols: '1.4fr 1fr', panels: [
    [{ t: 'Carrier', x: 'what the voice shapes', grow: true, s: [[['d', 'carrier', 230]], [['p', 'Carrier']]] }],
    [{ t: 'Tone', s: [[['k', 'Tone']]] }, { t: 'Follower', s: [[['k', 'Attack', 'Release']]] }, { t: 'Output', grow: true, s: [[['K', 'Wet']], [levels]] }],
  ] },
  stutter: { sub: 'FX · Stutter', fam: 'fx', w: 1000, cols: '1.4fr 1fr', mix: 'Mix', panels: [
    [{ t: 'Slices', x: 'repeat pattern', grow: true, s: [[['d', 'slices', 220]], [['p', 'Sync']]] }],
    [{ t: 'Stutter', s: [[['k', 'Slice', 'Repeats', 'Decay']]] }, { t: 'Output', grow: true, s: [[levels]] }],
  ] },
}

/** The EQ's bands, in order: a low shelf, five bells, a high shelf. */
export const EQ_BAND_KINDS = ['lowshelf', 'peak', 'peak', 'peak', 'peak', 'peak', 'highshelf'] as const
export const EQ_BAND_COLOURS = ['#2DD4BF', '#F59E0B', '#A78BFA', '#38BDF8', '#F472B6', '#A3E635', '#FB7185', '#FBBF24']

/** The built-in id ("hardwave.native.eq") to its layout key ("eq"). */
export function layoutFor(pluginId: string): PluginLayout | null {
  return LAYOUTS[pluginId.replace(/^hardwave\.native\./, '')] ?? null
}

/** Window width for a layout, with room for a mix or choice in its header. */
export function windowWidth(l: PluginLayout): number {
  return l.mix || l.hp ? Math.max(l.w, 1040) : l.w
}
