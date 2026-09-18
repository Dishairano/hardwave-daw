import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { invoke } from '@tauri-apps/api/core'

/**
 * Audio-engine behaviour switches, applied by the engine.
 *
 * These were UI-only for a long time: the toggles were stored here and the
 * engine never read them, so both behaved as whatever the audio thread
 * happened to do. They are now pushed into the engine, which shares them with
 * the audio thread as atomics, so a change applies to the next block.
 *
 *  - resetPluginsOnTransport: when on, every node in the graph resets on
 *    transport Stop and on a playhead jump, so held instrument voices stop
 *    instead of carrying on at the new position.
 *  - playTruncatedNotes: when on, a note the playhead landed in the middle of
 *    is played from its middle, with its envelope where it would have been.
 *    When off, only notes that start at or after the playhead are played.
 *  - recordOffsetMs: manual correction added to the measured recording
 *    latency, for interfaces that do not report all of theirs.
 */
interface AudioPrefsState {
  resetPluginsOnTransport: boolean
  playTruncatedNotes: boolean
  recordOffsetMs: number
  setResetPluginsOnTransport: (v: boolean) => void
  setPlayTruncatedNotes: (v: boolean) => void
  setRecordOffsetMs: (ms: number) => void
}

export const useAudioPrefsStore = create<AudioPrefsState>()(
  persist(
    (set) => ({
      resetPluginsOnTransport: true,
      playTruncatedNotes: false,
      recordOffsetMs: 0,
      setResetPluginsOnTransport: (resetPluginsOnTransport) => {
        set({ resetPluginsOnTransport })
        invoke('set_reset_on_transport', { enabled: resetPluginsOnTransport }).catch(() => {})
      },
      setPlayTruncatedNotes: (playTruncatedNotes) => {
        set({ playTruncatedNotes })
        invoke('set_play_truncated_notes', { enabled: playTruncatedNotes }).catch(() => {})
      },
      setRecordOffsetMs: (ms) => {
        const recordOffsetMs = Math.max(-500, Math.min(500, Math.round(Number.isFinite(ms) ? ms : 0)))
        set({ recordOffsetMs })
        invoke('set_record_offset_ms', { ms: recordOffsetMs }).catch(() => {})
      },
    }),
    { name: 'hw-audio-prefs' },
  ),
)

/**
 * Push the saved switches into the engine.
 *
 * They persist in localStorage, so after a restart the panel and the engine
 * disagree until something says otherwise. Called once from `App.tsx` on
 * boot, and safe to call again.
 */
export function applySavedAudioPrefs(): void {
  const { resetPluginsOnTransport, playTruncatedNotes, recordOffsetMs } = useAudioPrefsStore.getState()
  invoke('set_reset_on_transport', { enabled: resetPluginsOnTransport }).catch(() => {})
  invoke('set_play_truncated_notes', { enabled: playTruncatedNotes }).catch(() => {})
  invoke('set_record_offset_ms', { ms: recordOffsetMs ?? 0 }).catch(() => {})
}
