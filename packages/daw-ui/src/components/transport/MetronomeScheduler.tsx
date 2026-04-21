import { useEffect, useRef } from 'react'
import { useMetronomeStore } from '../../stores/metronomeStore'
import { useTransportStore } from '../../stores/transportStore'

// Plays a short sine beep through WebAudio. The engine runs in the native audio
// thread, so this click rides alongside it — there is a small device-offset
// between the two, but the beat-to-beat timing stays stable because we fire on
// each transport update crossing a new beat boundary.
function playClick(ctx: AudioContext, freq: number, volume: number) {
  const t = ctx.currentTime
  const osc = ctx.createOscillator()
  const gain = ctx.createGain()
  osc.type = 'sine'
  osc.frequency.value = freq
  const peak = Math.max(0.0001, volume) * 0.6
  gain.gain.setValueAtTime(0, t)
  gain.gain.linearRampToValueAtTime(peak, t + 0.002)
  gain.gain.exponentialRampToValueAtTime(0.0001, t + 0.055)
  osc.connect(gain)
  gain.connect(ctx.destination)
  osc.start(t)
  osc.stop(t + 0.08)
}

export function MetronomeScheduler() {
  const enabled = useMetronomeStore(s => s.enabled)
  const volume = useMetronomeStore(s => s.volume)
  const accent = useMetronomeStore(s => s.accent)
  const recordOnly = useMetronomeStore(s => s.recordOnly)

  const ctxRef = useRef<AudioContext | null>(null)
  const lastBeatRef = useRef<number>(-1)
  const lastPlayingRef = useRef<boolean>(false)

  useEffect(() => {
    if (!enabled) return
    const Ctor = (window as unknown as { AudioContext?: typeof AudioContext; webkitAudioContext?: typeof AudioContext }).AudioContext
      || (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
    if (!Ctor) return
    const ctx = new Ctor()
    ctxRef.current = ctx

    const unsub = useTransportStore.subscribe((s) => {
      const wasPlaying = lastPlayingRef.current
      lastPlayingRef.current = s.playing

      if (!s.playing) {
        lastBeatRef.current = -1
        return
      }

      const { recording } = useTransportStore.getState()
      if (recordOnly && !recording) return

      const sr = s.sampleRate || 48000
      const beat = Math.floor((s.positionSamples / sr) * (s.bpm / 60))

      // Fresh playback start: establish baseline without clicking.
      if (!wasPlaying || lastBeatRef.current < 0) {
        lastBeatRef.current = beat - 1
      }

      if (beat !== lastBeatRef.current) {
        const bpb = s.timeSigNumerator > 0 ? s.timeSigNumerator : 4
        const beatInBar = ((beat % bpb) + bpb) % bpb
        const freq = accent && beatInBar === 0 ? 1500 : 800
        if (ctx.state === 'suspended') ctx.resume().catch(() => {})
        playClick(ctx, freq, volume)
        lastBeatRef.current = beat
      }
    })

    return () => {
      unsub()
      try { ctx.close() } catch {}
      ctxRef.current = null
    }
  }, [enabled, volume, accent, recordOnly])

  return null
}
