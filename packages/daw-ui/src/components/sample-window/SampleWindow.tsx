/**
 * The sample window: a sample's channel settings, as FL has them, opened by
 * double-clicking an audio clip. Approved as version D of the mockup on
 * 13 October 2026 (suite: daw-sample-window-mockup): FL's layout and grid,
 * the DAW's own colours, flat.
 *
 * FL keeps these settings per channel, so they belong to the sample here:
 * a change reaches every clip that plays the same file, wherever it sits.
 * What the DAW cannot do yet is shown switched off, as FL greys what does
 * not apply, and says so when pointed at.
 */
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { invokeOrToast } from '../../api/invoke'
import { useSampleWindowStore } from '../../stores/sampleWindowStore'
import { useTrackStore, type ClipInfo } from '../../stores/trackStore'
import { useTransportStore } from '../../stores/transportStore'
import { useHistoryStore } from '../../stores/historyStore'
import { useProjectStore } from '../../stores/projectStore'
import { PPQ } from '../../utils/meter'
import { FlatBox, FlatCheck, FlatKnob, FlatPanel, FlatSelect, FlatSwitch } from '../ui/flat/Flat'
import './sampleWindow.css'

/** What the window shows and changes, in the units it shows them in. */
interface Values {
  muted: boolean
  gainDb: number
  pan: number
  pitch: number
  stretch: number
  resample: boolean
  reversed: boolean
  polarityInverted: boolean
  swapStereo: boolean
  normalize: boolean
  removeDc: boolean
  fadeInMs: number
  fadeOutMs: number
}

/** The command's own names for the same things. */
type Settings = Partial<{
  muted: boolean
  gainDb: number
  pan: number
  pitchSemitones: number
  stretchRatio: number
  resample: boolean
  reversed: boolean
  polarityInverted: boolean
  swapStereo: boolean
  normalize: boolean
  removeDc: boolean
  fadeInMs: number
  fadeOutMs: number
}>

function valuesOf(clip: ClipInfo): Values {
  const t = clip.treatment
  return {
    muted: clip.muted,
    gainDb: clip.gainDb,
    pan: t?.pan ?? 0,
    pitch: clip.pitchSemitones,
    stretch: clip.stretchRatio || 1,
    resample: t?.resample ?? false,
    reversed: clip.reversed,
    polarityInverted: t?.polarityInverted ?? false,
    swapStereo: t?.swapStereo ?? false,
    normalize: t?.normalize ?? false,
    removeDc: t?.removeDc ?? false,
    fadeInMs: clip.fadeInMs ?? 0,
    fadeOutMs: clip.fadeOutMs ?? 0,
  }
}

function settingsOf(v: Partial<Values>): Settings {
  const s: Settings = {}
  if (v.muted !== undefined) s.muted = v.muted
  if (v.gainDb !== undefined) s.gainDb = v.gainDb
  if (v.pan !== undefined) s.pan = v.pan
  if (v.pitch !== undefined) s.pitchSemitones = v.pitch
  if (v.stretch !== undefined) s.stretchRatio = v.stretch
  if (v.resample !== undefined) s.resample = v.resample
  if (v.reversed !== undefined) s.reversed = v.reversed
  if (v.polarityInverted !== undefined) s.polarityInverted = v.polarityInverted
  if (v.swapStereo !== undefined) s.swapStereo = v.swapStereo
  if (v.normalize !== undefined) s.normalize = v.normalize
  if (v.removeDc !== undefined) s.removeDc = v.removeDc
  if (v.fadeInMs !== undefined) s.fadeInMs = v.fadeInMs
  if (v.fadeOutMs !== undefined) s.fadeOutMs = v.fadeOutMs
  return s
}

const MODES = ['Resample', 'Stretch'] as const
const MODE_HINT: Record<(typeof MODES)[number], string> = {
  Resample: 'speed and pitch together, like a tape',
  Stretch: 'length and pitch apart',
}
const ms = (v: number) => (v <= 0 ? '0 ms' : v < 1000 ? `${v < 10 ? v.toFixed(1) : Math.round(v)} ms` : `${(v / 1000).toFixed(2)} s`)
const db = (v: number) => (v <= -60 ? '-inf dB' : `${v > 0 ? '+' : ''}${v.toFixed(1)} dB`)
const NOT_YET = 'Not in this version yet'
/** How much longer than its file the sample lasts: the stretch, and in
 *  resample also the pitch (AudioClip::time_factor). */
const timeFactor = (v: Values) => v.stretch / (v.resample ? Math.pow(2, v.pitch / 12) : 1)
const FOR_SAMPLER = 'For sampler channels; comes with them'

export function SampleWindow() {
  const target = useSampleWindowStore((s) => s.target)
  if (!target) return null
  return <SampleWindowFor key={`${target.trackId}/${target.clipId}`} trackId={target.trackId} clipId={target.clipId} />
}

function SampleWindowFor({ trackId, clipId }: { trackId: string; clipId: string }) {
  const close = useSampleWindowStore((s) => s.close)
  const tracks = useTrackStore((s) => s.tracks)
  const bpm = useTransportStore((s) => s.bpm)
  const colorOverride = useTransportStore((s) => s.clipColorOverrides[clipId])
  const track = tracks.find((t) => t.id === trackId)
  const clip = track?.clips.find((c) => c.id === clipId)

  const [v, setV] = useState<Values | null>(() => (clip ? valuesOf(clip) : null))
  const [range, setRange] = useState(() => Math.max(2, Math.ceil(Math.abs(clip?.pitchSemitones ?? 0))))
  const gesture = useRef(false)
  // What arrives from the store wins, except in the middle of a gesture.
  useEffect(() => {
    if (clip && !gesture.current) setV(valuesOf(clip))
  }, [clip])

  // The window sits where it was dragged; it opens near the top, centred.
  const [pos, setPos] = useState(() => ({ x: Math.max(16, (window.innerWidth - 720) / 2), y: 96 }))
  useEffect(() => {
    const esc = (e: KeyboardEvent) => { if (e.key === 'Escape') close() }
    window.addEventListener('keydown', esc)
    return () => window.removeEventListener('keydown', esc)
  }, [close])

  // A change: shown at once, sent to the engine. A gesture's first move
  // takes the undo snapshot; its end reloads the song and names the step.
  const sendTimer = useRef<number | null>(null)
  const pending = useRef<Settings>({})
  const flush = (snapshot: boolean) => {
    const settings = pending.current
    pending.current = {}
    if (sendTimer.current !== null) { window.clearTimeout(sendTimer.current); sendTimer.current = null }
    if (Object.keys(settings).length === 0) return Promise.resolve()
    return invokeOrToast('set_sample_settings', { trackId, clipId, settings, snapshot }, { message: 'Changing the sample failed', retry: false })
  }
  const change = (patch: Partial<Values>, done: boolean, label: string) => {
    // A click on a knob that did not turn it is not a change.
    const same = v !== null && Object.entries(patch).every(([k, val]) => v[k as keyof Values] === val)
    if (done && !gesture.current && same) return
    setV((prev) => (prev ? { ...prev, ...patch } : prev))
    pending.current = { ...pending.current, ...settingsOf(patch) }
    const first = !gesture.current
    if (!done) {
      gesture.current = true
      if (first) { void flush(true); return }
      if (sendTimer.current === null) sendTimer.current = window.setTimeout(() => { void flush(false) }, 40)
      return
    }
    gesture.current = false
    void flush(first).then(async () => {
      useProjectStore.getState().markDirty()
      useHistoryStore.getState().push(label)
      await useTrackStore.getState().fetchTracks()
    })
  }

  if (!track || !clip || !v) return null

  const color = colorOverride || track.color || '#10b981'
  const seconds = clip.sourceFrames && clip.sourceRate ? clip.sourceFrames / clip.sourceRate : 0
  const beatSec = 60 / Math.max(1, bpm)
  const naturalBeats = seconds / beatSec
  const beatsNow = naturalBeats * timeFactor(v)
  const fileName = (clip.sourceFile || clip.source_id).split(/[\\/]/).pop() || clip.name
  const output = track.outputBus ? tracks.find((t) => t.id === track.outputBus) : null
  const trackNumber = tracks.filter((t) => t.kind !== 'Master').findIndex((t) => t.id === track.id) + 1

  return (
    <div className="hwf sw-win" style={{ left: pos.x, top: pos.y }} role="dialog" aria-label={`${fileName} settings`}>
      <div
        className="sw-title"
        onPointerDown={(e) => {
          if ((e.target as HTMLElement).closest('button')) return
          const x0 = e.clientX - pos.x
          const y0 = e.clientY - pos.y
          const move = (ev: PointerEvent) => setPos({
            x: Math.max(0, Math.min(window.innerWidth - 120, ev.clientX - x0)),
            y: Math.max(28, Math.min(window.innerHeight - 40, ev.clientY - y0)),
          })
          const up = () => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up) }
          window.addEventListener('pointermove', move)
          window.addEventListener('pointerup', up)
        }}
      >
        <span className="sw-swatch" style={{ background: color }} />
        <span className="sw-name">{clip.name || fileName}</span>
        <span className="sw-grow" />
        <button type="button" className="sw-x" title="Close (Esc)" onClick={close}>✕</button>
      </div>

      <div className="sw-top">
        <div className="sw-tab is-on" title="Sample"><svg width="22" height="16" viewBox="0 0 30 22" fill="none" strokeWidth="2.2" strokeLinecap="round"><path d="M2 11h4l2-5 3 10 3-14 3 18 3-12 2 6 2-3h4" /></svg></div>
        <div className="sw-tab is-off" title={`Envelope. ${FOR_SAMPLER}`}><svg width="20" height="18" viewBox="0 0 26 24" fill="none" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><circle cx="4" cy="20" r="2.5" /><circle cx="22" cy="20" r="2.5" /><path d="M5.5 18 12 3l3 7 5.5 8" /></svg></div>
        <div className="sw-tab is-off" title={`More. ${NOT_YET}`}><svg width="18" height="18" viewBox="0 0 24 24" fill="none" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M14.5 6.5a4.5 4.5 0 0 0-6 5.5L3 17.5 6.5 21 12 15.5a4.5 4.5 0 0 0 5.5-6l-3 3-2.5-.5-.5-2.5z" /></svg></div>
        <span className="sw-grow" />
        <FlatSwitch label="On" on={!v.muted} onToggle={() => change({ muted: !v.muted }, true, v.muted ? 'Sample on' : 'Sample off')} />
        <FlatKnob
          label="Pan" size={32} value={(v.pan + 1) / 2} def={0.5}
          text={`Pan ${v.pan === 0 ? 'centred' : `${Math.round(Math.abs(v.pan) * 100)}% ${v.pan < 0 ? 'left' : 'right'}`}`}
          onChange={(t, done) => change({ pan: Math.round((t * 2 - 1) * 100) / 100 }, done, 'Pan sample')}
          onReset={() => change({ pan: 0 }, true, 'Pan sample')}
        />
        <FlatKnob
          label="Vol" size={32} value={(v.gainDb + 60) / 72} def={60 / 72}
          text={`Volume ${db(v.gainDb)}`}
          onChange={(t, done) => change({ gainDb: Math.round((t * 72 - 60) * 10) / 10 }, done, 'Sample volume')}
          onReset={() => change({ gainDb: 0 }, true, 'Sample volume')}
        />
        <FlatKnob
          label="Pitch" size={32} value={(v.pitch + range) / (2 * range)} def={0.5}
          text={`Pitch ${v.pitch > 0 ? '+' : ''}${v.pitch.toFixed(2)} st`}
          onChange={(t, done) => change({ pitch: Math.round((t * 2 - 1) * range * 100) / 100 }, done, 'Pitch sample')}
          onReset={() => change({ pitch: 0 }, true, 'Pitch sample')}
        />
        <FlatBox label="Range" value={range} onStep={(by) => setRange((r) => Math.max(1, Math.min(48, r + by)))} hint={`The pitch knob turns ${range} semitones each way. Scroll to change.`} />
        <FlatBox label="Track" value={trackNumber || '-'} small={output?.name ?? 'Master'} hint={`${track.name} plays into ${output?.name ?? 'the master'}; route it in the mixer`} />
      </div>

      <div className="sw-cols">
        <div className="sw-col">
          <FlatPanel>
            <div className="sw-file">
              <span className="hwf-lab" style={{ textTransform: 'none', fontSize: 12 }}>File</span>
              <span className="sw-file-name" title={clip.sourceFile || clip.source_id}>{fileName}</span>
            </div>
          </FlatPanel>
          <FlatPanel title="Content">
            <div className="hwf-checks">
              <FlatCheck label="Keep on disk" on={false} disabled hint={NOT_YET} />
              <FlatCheck label="Load regions" on={false} disabled hint={NOT_YET} />
              <FlatCheck label="Resample" on={false} disabled hint={NOT_YET} />
              <FlatCheck label="Load slice markers" on={false} disabled hint={NOT_YET} />
            </div>
            <div className="sw-row">
              <span className="sw-lead">Declicking mode</span>
              <FlatSelect value="Out only" options={['Out only'] as const} disabled style={{ flex: 1 }} />
            </div>
          </FlatPanel>
          <FlatPanel title="Playback" grow>
            <div className="hwf-krow">
              <FlatKnob label="Offset" value={0} def={0} text="" disabled offText={FOR_SAMPLER} />
              <FlatKnob label="Loop start" value={0} def={0} text="" disabled offText={FOR_SAMPLER} />
              <FlatKnob label="Loop end" value={1} def={1} text="" disabled offText={FOR_SAMPLER} />
            </div>
            <div className="hwf-checks">
              <FlatCheck label="Use loop points" on={false} disabled hint={FOR_SAMPLER} />
              <FlatCheck label="Ping pong loop" on={false} disabled hint={FOR_SAMPLER} />
            </div>
          </FlatPanel>
        </div>

        <div className="sw-col">
          <FlatPanel title="Time stretching">
            <div className="sw-row" style={{ alignItems: 'flex-start' }}>
              <div className="hwf-krow" style={{ flex: '0 0 192px' }}>
                <FlatKnob
                  label="Pitch" value={(v.pitch * 100 + 1200) / 2400} def={0.5}
                  text={`Pitch ${v.pitch > 0 ? '+' : ''}${Math.round(v.pitch * 100)} cents (the pitch at the top)`}
                  onChange={(t, done) => change({ pitch: Math.round(t * 2400 - 1200) / 100 }, done, 'Pitch sample')}
                  onReset={() => change({ pitch: 0 }, true, 'Pitch sample')}
                />
                <FlatKnob
                  label="Mul" value={(Math.log2(v.stretch) + 2) / 4} def={0.5}
                  text={`Length × ${v.stretch.toFixed(2)}`}
                  onChange={(t, done) => change({ stretch: Math.round(Math.pow(2, t * 4 - 2) * 100) / 100 }, done, 'Stretch sample')}
                  onReset={() => change({ stretch: 1 }, true, 'Stretch sample')}
                />
                <FlatKnob
                  label="Time" value={Math.min(1, beatsNow / 64)} def={Math.min(1, naturalBeats / 64)}
                  disabled={!seconds} offText="The file is not loaded"
                  text={`Lasts ${beatsNow.toFixed(beatsNow % 1 ? 2 : 0)} beats`}
                  onChange={(t, done) => {
                    // Whole quarter beats, as FL's TIME knob counts.
                    const beats = Math.max(0.25, Math.round(t * 64 * 4) / 4)
                    const pitchSpeed = v.resample ? Math.pow(2, v.pitch / 12) : 1
                    change({ stretch: Math.max(0.25, Math.min(4, (beats / naturalBeats) * pitchSpeed)) }, done, 'Fit sample to time')
                  }}
                  onReset={() => change({ stretch: 1 }, true, 'Fit sample to time')}
                />
              </div>
              <div className="sw-stack">
                <span className="hwf-lab">Mode</span>
                <FlatSelect
                  value={v.resample ? 'Resample' : 'Stretch'} options={MODES}
                  onPick={(m) => change({ resample: m === 'Resample' }, true, `Stretch mode ${m}`)}
                />
                <span className="sw-hint">{MODE_HINT[v.resample ? 'Resample' : 'Stretch']}</span>
              </div>
            </div>
          </FlatPanel>
          <FlatPanel title="Precomputed effects" grow>
            <div className="hwf-checks">
              <FlatCheck label="Remove DC offset" on={v.removeDc} onToggle={() => change({ removeDc: !v.removeDc }, true, 'Remove DC offset')} />
              <FlatCheck label="Reverse polarity" on={v.polarityInverted} onToggle={() => change({ polarityInverted: !v.polarityInverted }, true, 'Reverse polarity')} />
              <FlatCheck label="Normalize" on={v.normalize} onToggle={() => change({ normalize: !v.normalize }, true, 'Normalize sample')} />
              <FlatCheck label="Fade stereo" on={false} disabled hint={NOT_YET} />
              <FlatCheck label="Reverse" on={v.reversed} onToggle={() => change({ reversed: !v.reversed }, true, 'Reverse sample')} />
              <FlatCheck label="Swap stereo" on={v.swapStereo} onToggle={() => change({ swapStereo: !v.swapStereo }, true, 'Swap stereo')} />
            </div>
            <div className="hwf-krow">
              <FlatKnob label="Start" value={0} def={0} text="" disabled />
              <FlatKnob label="Length" value={1} def={1} text="" disabled />
              <FlatKnob
                label="In" value={Math.sqrt(v.fadeInMs / 2000)} def={0}
                text={`Fade in ${ms(v.fadeInMs)}`}
                onChange={(t, done) => change({ fadeInMs: Math.round(t * t * 2000) }, done, 'Sample fade in')}
                onReset={() => change({ fadeInMs: 0 }, true, 'Sample fade in')}
              />
              <FlatKnob
                label="Out" value={Math.sqrt(v.fadeOutMs / 2000)} def={0}
                text={`Fade out ${ms(v.fadeOutMs)}`}
                onChange={(t, done) => change({ fadeOutMs: Math.round(t * t * 2000) }, done, 'Sample fade out')}
                onReset={() => change({ fadeOutMs: 0 }, true, 'Sample fade out')}
              />
              <FlatKnob label="X-fade" value={0} def={0} text="" disabled offText={FOR_SAMPLER} />
              <FlatKnob label="Trim" value={0} def={0} text="" disabled />
            </div>
          </FlatPanel>
        </div>
      </div>

      <SampleView clip={clip} values={v} color={color} bpm={bpm} seconds={seconds} />
    </div>
  )
}

/** The sample, wide at the bottom: what this clip plays of it, its fades,
 *  the song's bars over it, and an overview strip under it. */
function SampleView({ clip, values, color, bpm, seconds }: { clip: ClipInfo; values: Values; color: string; bpm: number; seconds: number }) {
  const cv = useRef<HTMLCanvasElement>(null)
  const overview = useRef<HTMLCanvasElement>(null)
  const [peaks, setPeaks] = useState<number[][]>([])
  useEffect(() => {
    let live = true
    invoke<number[][]>('get_waveform_peaks', { sourceId: clip.source_id, numBuckets: 700 })
      .then((p) => { if (live) setPeaks(p) })
      .catch(() => { if (live) setPeaks([]) })
    return () => { live = false }
  }, [clip.source_id])

  const region = useMemo(() => {
    const rate = clip.sourceRate || 48_000
    const start = (clip.sourceStart ?? 0) / rate
    const playsFor = (clip.length_ticks / PPQ) * (60 / Math.max(1, bpm)) / Math.max(0.01, timeFactor(values))
    return { start, end: Math.min(seconds || start + playsFor, start + playsFor) }
  }, [clip.sourceRate, clip.sourceStart, clip.length_ticks, bpm, values, seconds])

  useLayoutEffect(() => {
    const c = cv.current
    if (!c) return
    const dpr = window.devicePixelRatio || 1
    const w = c.clientWidth
    const h = c.clientHeight
    c.width = w * dpr
    c.height = h * dpr
    const g = c.getContext('2d')
    if (!g) return
    g.scale(dpr, dpr)
    const ruler = 18
    const total = seconds || Math.max(region.end, 1)
    const X = (t: number) => (t / total) * w
    g.fillStyle = '#0e0e10'; g.fillRect(0, 0, w, ruler)
    g.fillStyle = '#0a0a0b'; g.fillRect(0, ruler, w, h - ruler)
    g.fillStyle = 'rgba(255,255,255,.08)'; g.fillRect(0, ruler - 1, w, 1)
    const beat = 60 / Math.max(1, bpm)
    g.font = '600 11px Inter, sans-serif'
    for (let t = 0, b = 0; t < total; t += beat, b++) {
      const x = Math.round(X(t)) + 0.5
      g.strokeStyle = b % 4 === 0 ? 'rgba(255,255,255,.08)' : 'rgba(255,255,255,.035)'
      g.beginPath(); g.moveTo(x, ruler); g.lineTo(x, h); g.stroke()
      if (b % 4 === 0) { g.fillStyle = '#74747e'; g.fillText(String(b / 4 + 1), x + 4, 13) }
    }
    const mid = ruler + (h - ruler) / 2
    const half = (h - ruler) / 2 - 4
    const n = peaks.length
    for (let i = 0; i < n; i++) {
      const p = values.reversed ? peaks[n - 1 - i] : peaks[i]
      const t = ((i + 0.5) / n) * total
      const inside = t >= region.start && t <= region.end
      g.fillStyle = inside ? color : 'rgba(168,168,178,.18)'
      g.globalAlpha = inside ? 0.9 : 1
      const top = mid - Math.max(0, p[1]) * half
      const bottom = mid - Math.min(0, p[0]) * half
      g.fillRect((i / n) * w, top, Math.max(1, w / n + 0.4), Math.max(1, bottom - top))
    }
    g.globalAlpha = 1
    g.fillStyle = 'rgba(10,10,11,.6)'
    g.fillRect(0, ruler, X(region.start), h - ruler)
    g.fillRect(X(region.end), ruler, w - X(region.end), h - ruler)
    g.strokeStyle = 'rgba(236,236,240,.7)'; g.lineWidth = 1.2
    const fin = values.fadeInMs / 1000 / Math.max(0.01, timeFactor(values))
    const fout = values.fadeOutMs / 1000 / Math.max(0.01, timeFactor(values))
    if (fin > 0) { g.beginPath(); g.moveTo(X(region.start), h); g.lineTo(X(region.start + fin), ruler); g.stroke() }
    if (fout > 0) { g.beginPath(); g.moveTo(X(region.end - fout), ruler); g.lineTo(X(region.end), h); g.stroke() }
    g.lineWidth = 1
    const sx = Math.round(X(region.start)) + 0.5
    g.fillStyle = '#ececf0'
    g.fillRect(sx - 0.5, ruler, 1.5, h - ruler)
    g.beginPath(); g.moveTo(sx - 0.5, ruler); g.lineTo(sx + 7, ruler); g.lineTo(sx - 0.5, ruler + 8); g.closePath(); g.fill()
    g.fillText('Start', sx + 10, ruler + 12)
    if (seconds) {
      g.fillStyle = '#74747e'; g.font = '500 11px Inter, sans-serif'; g.textAlign = 'right'
      g.fillText(`${(seconds / beat).toFixed(1)} beats at ${Math.round(bpm)} BPM`, w - 10, 13)
      g.textAlign = 'left'
    }
    if (!n) { g.fillStyle = '#74747e'; g.font = '500 12px Inter, sans-serif'; g.fillText('The file is not loaded', 12, mid + 4) }

    const o = overview.current
    if (!o) return
    const ow = o.clientWidth
    const oh = o.clientHeight
    o.width = ow * dpr
    o.height = oh * dpr
    const og = o.getContext('2d')
    if (!og) return
    og.scale(dpr, dpr)
    og.fillStyle = '#1b1b20'; og.fillRect(0, 4, ow, oh - 8)
    og.fillStyle = color; og.globalAlpha = 0.55
    for (let i = 0; i < n; i++) {
      const a = Math.max(Math.abs(peaks[i][0]), Math.abs(peaks[i][1])) * (oh / 2 - 5)
      og.fillRect((i / n) * ow, oh / 2 - a, Math.max(1, ow / n), a * 2)
    }
  }, [peaks, values, region, color, bpm, seconds])

  return (
    <div className="sw-view">
      <canvas ref={cv} className="sw-wave" />
      <canvas ref={overview} className="sw-overview" />
    </div>
  )
}
