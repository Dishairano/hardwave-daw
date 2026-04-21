import { useEffect, useRef } from 'react'
import { useMetronomeStore } from '../../stores/metronomeStore'
import { useTransportStore } from '../../stores/transportStore'

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

function playSample(ctx: AudioContext, buffer: AudioBuffer, volume: number) {
  const t = ctx.currentTime
  const src = ctx.createBufferSource()
  const gain = ctx.createGain()
  src.buffer = buffer
  gain.gain.value = Math.max(0.0001, volume) * 1.2
  src.connect(gain)
  gain.connect(ctx.destination)
  src.start(t)
}

async function dataUrlToAudioBuffer(ctx: AudioContext, dataUrl: string): Promise<AudioBuffer | null> {
  try {
    const resp = await fetch(dataUrl)
    const buf = await resp.arrayBuffer()
    return await ctx.decodeAudioData(buf)
  } catch {
    return null
  }
}

export function MetronomeScheduler() {
  const enabled = useMetronomeStore(s => s.enabled)
  const volume = useMetronomeStore(s => s.volume)
  const accent = useMetronomeStore(s => s.accent)
  const recordOnly = useMetronomeStore(s => s.recordOnly)
  const customDownbeat = useMetronomeStore(s => s.customDownbeat)
  const customAccent = useMetronomeStore(s => s.customAccent)

  const ctxRef = useRef<AudioContext | null>(null)
  const lastBeatRef = useRef<number>(-1)
  const lastPlayingRef = useRef<boolean>(false)
  const downbeatBufRef = useRef<AudioBuffer | null>(null)
  const accentBufRef = useRef<AudioBuffer | null>(null)

  useEffect(() => {
    if (!enabled) return
    const Ctor = (window as unknown as { AudioContext?: typeof AudioContext; webkitAudioContext?: typeof AudioContext }).AudioContext
      || (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
    if (!Ctor) return
    const ctx = new Ctor()
    ctxRef.current = ctx

    let cancelled = false
    const loadBuffers = async () => {
      if (customDownbeat) {
        const buf = await dataUrlToAudioBuffer(ctx, customDownbeat)
        if (!cancelled) downbeatBufRef.current = buf
      } else {
        downbeatBufRef.current = null
      }
      if (customAccent) {
        const buf = await dataUrlToAudioBuffer(ctx, customAccent)
        if (!cancelled) accentBufRef.current = buf
      } else {
        accentBufRef.current = null
      }
    }
    loadBuffers()

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

      if (!wasPlaying || lastBeatRef.current < 0) {
        lastBeatRef.current = beat - 1
      }

      if (beat !== lastBeatRef.current) {
        const bpb = s.timeSigNumerator > 0 ? s.timeSigNumerator : 4
        const beatInBar = ((beat % bpb) + bpb) % bpb
        const isDownbeat = accent && beatInBar === 0
        if (ctx.state === 'suspended') ctx.resume().catch(() => {})

        const buf = isDownbeat
          ? (accentBufRef.current ?? downbeatBufRef.current)
          : downbeatBufRef.current
        if (buf) {
          playSample(ctx, buf, volume)
        } else {
          const freq = isDownbeat ? 1500 : 800
          playClick(ctx, freq, volume)
        }
        lastBeatRef.current = beat
      }
    })

    return () => {
      cancelled = true
      unsub()
      try { ctx.close() } catch {}
      ctxRef.current = null
    }
  }, [enabled, volume, accent, recordOnly, customDownbeat, customAccent])

  return null
}
