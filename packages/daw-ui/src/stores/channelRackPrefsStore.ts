import { create } from 'zustand'
import { persist } from 'zustand/middleware'

/**
 * FL Studio-style Channel Rack preferences shared across the whole
 * rack: global swing multiplier, pattern length override (1..512
 * steps), per-channel loop mode (Bar / Beat / Step / Off), per-
 * channel zipped state, and the boolean toggles that FL exposes
 * through the Channel Options right-click menu.
 *
 * Frontend-only for now — these flags drive UI rendering and persist
 * to localStorage. The engine wiring (swing multiplier into step
 * timing, ghost-step preview into rendered patterns, etc.) lands in
 * follow-up batches.
 *
 *  globalSwing      0..1, multiplied with each channel's per-channel
 *                   swing (legacy `swing` const in ChannelRack) to
 *                   produce the effective odd-step delay.
 *  patternLength    Optional override of STEPS_PER_PATTERN — when
 *                   set, the rack renders up to N steps per row even
 *                   if the active pattern has shorter velocity arrays.
 *  loopMode         Map<channelId, 'off' | 'bar' | 'beat' | 'step'>.
 *                   When a channel's mode is non-'off', the rack
 *                   renders ghost-step indicators for steps beyond
 *                   the loop boundary.
 *  zipped           Set<channelId>. Channels in this set render as
 *                   the 18px collapsed row instead of the full 32px
 *                   row + step grid.
 *  showMixerSelectors  When true, the rack shows a mixer-track pill
 *                   between volume knob and step grid. When false,
 *                   it's hidden (the routing still works via the
 *                   channel button RMB → Mixer track sub-menu).
 *  showCompletePianoRoll  When true, the piano-roll mini preview on
 *                   melody channels shows ALL notes; when false, only
 *                   notes inside the current pattern bar range.
 *  muteRemovedSteps   When true, removing a step in the grid only
 *                   silences it (preserving the step data); when
 *                   false, removal nukes the step value.
 *  colorfulLoopControls  When true, loop-mode glyphs adopt the
 *                   channel button's color; when false they use a
 *                   neutral amber.
 *  showAdvancedLoopControls  When true, the Bar/Beat/Step glyph row
 *                   is always rendered next to each channel button;
 *                   when false, the row hides until Loop mode is on.
 *  autoZipEmpty     When true, channels with zero active steps
 *                   auto-collapse to the 18px zipped row.
 */

export type LoopMode = 'off' | 'bar' | 'beat' | 'step'

interface ChannelRackPrefsState {
  globalSwing: number
  setGlobalSwing: (v: number) => void

  patternLength: number | null
  setPatternLength: (n: number | null) => void

  loopMode: Record<string, LoopMode>
  setLoopMode: (channelId: string, mode: LoopMode) => void
  /** Loop step channels — global toggle independent of per-channel modes. */
  loopStepChannels: boolean
  toggleLoopStepChannels: () => void
  /** Loop all channels — when ON, looping applies to both step and PR. */
  loopAllChannels: boolean
  toggleLoopAllChannels: () => void

  zipped: string[]
  zip: (channelId: string) => void
  unzip: (channelId: string) => void
  zipMany: (channelIds: string[]) => void
  unzipAll: () => void
  isZipped: (channelId: string) => boolean

  showMixerSelectors: boolean
  toggleShowMixerSelectors: () => void
  showCompletePianoRoll: boolean
  toggleShowCompletePianoRoll: () => void
  muteRemovedSteps: boolean
  toggleMuteRemovedSteps: () => void
  colorfulLoopControls: boolean
  toggleColorfulLoopControls: () => void
  showAdvancedLoopControls: boolean
  toggleShowAdvancedLoopControls: () => void
  autoZipEmpty: boolean
  toggleAutoZipEmpty: () => void
  focusSelectedOnPlayback: boolean
  toggleFocusSelectedOnPlayback: () => void

  /** Per-channel swingmix multiplier (0..1). When zero the channel ignores swing entirely. */
  channelSwingmix: Record<string, number>
  setChannelSwingmix: (channelId: string, v: number) => void
  /** Per-channel "truncate swing notes" — prevents swung note overlap. */
  truncateSwing: Record<string, boolean>
  toggleTruncateSwing: (channelId: string) => void
}

export const useChannelRackPrefsStore = create<ChannelRackPrefsState>()(
  persist(
    (set, get) => ({
      globalSwing: 0,
      setGlobalSwing: (v) => set({ globalSwing: Math.max(0, Math.min(1, v)) }),

      patternLength: null,
      setPatternLength: (n) => set({ patternLength: n == null ? null : Math.max(1, Math.min(512, Math.round(n))) }),

      loopMode: {},
      setLoopMode: (channelId, mode) => set(s => ({ loopMode: { ...s.loopMode, [channelId]: mode } })),
      loopStepChannels: true,
      toggleLoopStepChannels: () => set(s => ({ loopStepChannels: !s.loopStepChannels })),
      loopAllChannels: false,
      toggleLoopAllChannels: () => set(s => ({ loopAllChannels: !s.loopAllChannels })),

      zipped: [],
      zip: (channelId) => set(s => s.zipped.includes(channelId) ? s : ({ zipped: [...s.zipped, channelId] })),
      unzip: (channelId) => set(s => ({ zipped: s.zipped.filter(id => id !== channelId) })),
      zipMany: (channelIds) => set(s => ({ zipped: Array.from(new Set([...s.zipped, ...channelIds])) })),
      unzipAll: () => set({ zipped: [] }),
      isZipped: (channelId) => get().zipped.includes(channelId),

      showMixerSelectors: true,
      toggleShowMixerSelectors: () => set(s => ({ showMixerSelectors: !s.showMixerSelectors })),
      showCompletePianoRoll: true,
      toggleShowCompletePianoRoll: () => set(s => ({ showCompletePianoRoll: !s.showCompletePianoRoll })),
      muteRemovedSteps: false,
      toggleMuteRemovedSteps: () => set(s => ({ muteRemovedSteps: !s.muteRemovedSteps })),
      colorfulLoopControls: true,
      toggleColorfulLoopControls: () => set(s => ({ colorfulLoopControls: !s.colorfulLoopControls })),
      showAdvancedLoopControls: false,
      toggleShowAdvancedLoopControls: () => set(s => ({ showAdvancedLoopControls: !s.showAdvancedLoopControls })),
      autoZipEmpty: false,
      toggleAutoZipEmpty: () => set(s => ({ autoZipEmpty: !s.autoZipEmpty })),
      focusSelectedOnPlayback: false,
      toggleFocusSelectedOnPlayback: () => set(s => ({ focusSelectedOnPlayback: !s.focusSelectedOnPlayback })),

      channelSwingmix: {},
      setChannelSwingmix: (channelId, v) => set(s => ({
        channelSwingmix: { ...s.channelSwingmix, [channelId]: Math.max(0, Math.min(1, v)) },
      })),
      truncateSwing: {},
      toggleTruncateSwing: (channelId) => set(s => ({
        truncateSwing: { ...s.truncateSwing, [channelId]: !s.truncateSwing[channelId] },
      })),
    }),
    { name: 'hw-channel-rack-prefs' },
  ),
)
