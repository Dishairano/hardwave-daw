/**
 * Keeps the engine's click in step with the settings, custom sounds included.
 *
 * The click lives in the engine, which places each beat on its own sample.
 * Custom click samples used to be the exception: this component watched the
 * playhead from React and played them through WebAudio, on a second clock, so
 * they drifted against the song and never reached the count-in. Now the
 * engine loads the chosen files and plays them like its own click.
 */
import { useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useMetronomeStore } from '../../stores/metronomeStore'
import { useNotificationStore } from '../../stores/notificationStore'

export function MetronomeScheduler() {
  const enabled = useMetronomeStore(s => s.enabled)
  const volume = useMetronomeStore(s => s.volume)
  const accent = useMetronomeStore(s => s.accent)
  const recordOnly = useMetronomeStore(s => s.recordOnly)
  const beatSound = useMetronomeStore(s => s.beatSound)
  const downbeatSound = useMetronomeStore(s => s.downbeatSound)

  useEffect(() => { invoke('set_metronome_enabled', { enabled }).catch(() => {}) }, [enabled])
  useEffect(() => { invoke('set_metronome_volume', { volume }).catch(() => {}) }, [volume])
  useEffect(() => { invoke('set_metronome_accent', { accent }).catch(() => {}) }, [accent])
  useEffect(() => {
    invoke('set_metronome_record_only', { recordOnly }).catch(() => {})
  }, [recordOnly])
  useEffect(() => {
    invoke('set_metronome_sound_files', { beat: beatSound, downbeat: downbeatSound }).catch((e) => {
      // A file that moved or will not decode leaves the click as it was;
      // say which one.
      useNotificationStore.getState().push('warning', 'The click sound could not be loaded', {
        detail: String(e),
      })
    })
  }, [beatSound, downbeatSound])

  return null
}
