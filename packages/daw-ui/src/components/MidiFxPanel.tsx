import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'
import { useTrackStore } from '../stores/trackStore'
import { useNotificationStore } from '../stores/notificationStore'

/**
 * MIDI effects between the clips and the instrument.
 *
 * The arpeggiator, chord maker and scale snapper used to be tools that
 * rewrote the notes in a clip: the part on screen stopped matching what
 * was played, and taking the effect off meant undoing. Here the clip
 * keeps its notes and the chain decides what the instrument hears, so
 * an effect can be switched off again.
 */

type ArpMode = 'Up' | 'Down' | 'UpDown' | 'AsPlayed'
type ScaleKind = 'Major' | 'NaturalMinor' | 'HarmonicMinor' | 'PhrygianDominant' | 'Pentatonic'

export type MidiFx =
  | { Arpeggiator: { step_ticks: number; mode: ArpMode; gate: number; octaves: number } }
  | { Chord: { intervals: number[] } }
  | { Scale: { root: number; kind: ScaleKind } }
  | { Transpose: { semitones: number } }

const PPQ = 960

const STEP_CHOICES: { label: string; ticks: number }[] = [
  { label: '1/4', ticks: PPQ },
  { label: '1/8', ticks: PPQ / 2 },
  { label: '1/8T', ticks: PPQ / 3 },
  { label: '1/16', ticks: PPQ / 4 },
  { label: '1/16T', ticks: PPQ / 6 },
  { label: '1/32', ticks: PPQ / 8 },
]

const CHORD_CHOICES: { label: string; intervals: number[] }[] = [
  { label: 'Minor', intervals: [3, 7] },
  { label: 'Major', intervals: [4, 7] },
  { label: 'Power', intervals: [7, 12] },
  { label: 'Minor 7', intervals: [3, 7, 10] },
  { label: 'Sus 4', intervals: [5, 7] },
  { label: 'Octave', intervals: [12] },
]

const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B']

const SCALE_LABELS: Record<ScaleKind, string> = {
  Major: 'Major',
  NaturalMinor: 'Minor',
  HarmonicMinor: 'Harmonic minor',
  PhrygianDominant: 'Phrygian dominant',
  Pentatonic: 'Pentatonic',
}

function label(fx: MidiFx): string {
  if ('Arpeggiator' in fx) return 'Arpeggiator'
  if ('Chord' in fx) return 'Chord'
  if ('Scale' in fx) return 'Scale'
  return 'Transpose'
}

export function MidiFxPanel({ trackId, onClose }: { trackId: string; onClose: () => void }) {
  const tracks = useTrackStore(s => s.tracks)
  const track = tracks.find(t => t.id === trackId)
  const [chain, setChain] = useState<MidiFx[]>([])
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    invoke<MidiFx[]>('get_midi_fx', { trackId })
      .then(list => { setChain(list); setLoaded(true) })
      .catch(() => setLoaded(true))
  }, [trackId])

  // Every change is sent straight away: an effect chain is something you
  // hear your way to, not something you fill in and submit.
  const push = useCallback(async (next: MidiFx[]) => {
    setChain(next)
    try {
      await invoke('set_midi_fx', { trackId, chain: next })
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not change the chain', { detail: String(e) })
    }
  }, [trackId])

  const add = useCallback((kind: string) => {
    const made: MidiFx =
      kind === 'arp' ? { Arpeggiator: { step_ticks: PPQ / 4, mode: 'Up', gate: 0.9, octaves: 1 } }
      : kind === 'chord' ? { Chord: { intervals: [3, 7] } }
      : kind === 'scale' ? { Scale: { root: 0, kind: 'NaturalMinor' } }
      : { Transpose: { semitones: 12 } }
    void push([...chain, made])
  }, [chain, push])

  const replace = useCallback((index: number, fx: MidiFx) => {
    void push(chain.map((x, i) => (i === index ? fx : x)))
  }, [chain, push])

  const remove = useCallback((index: number) => {
    void push(chain.filter((_, i) => i !== index))
  }, [chain, push])

  const move = useCallback((index: number, by: number) => {
    const target = index + by
    if (target < 0 || target >= chain.length) return
    const next = [...chain]
    const [taken] = next.splice(index, 1)
    next.splice(target, 0, taken)
    void push(next)
  }, [chain, push])

  return (
    <div
      style={{
        position: 'fixed', inset: 0, zIndex: 9800,
        background: 'rgba(0,0,0,0.45)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
      onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}
    >
      <div style={{
        width: 620, maxWidth: '95vw', maxHeight: '80vh',
        background: hw.bg, color: hw.textPrimary,
        border: `1px solid ${hw.border}`, borderRadius: hw.radius.lg,
        overflow: 'hidden', display: 'flex', flexDirection: 'column',
      }}>
        <div style={{
          padding: '8px 12px', display: 'flex', alignItems: 'center', gap: 12,
          background: hw.bgElevated, borderBottom: `1px solid ${hw.border}`,
        }}>
          <div style={{ fontSize: 12, fontWeight: 600 }}>
            MIDI effects · {track?.name ?? 'track'}
          </div>
          <div style={{ fontSize: 9, color: hw.textFaint }}>
            between the notes you wrote and the instrument
          </div>
          <div style={{ flex: 1 }} />
          <button onClick={onClose} style={btn()}>Close</button>
        </div>

        <div style={{ overflowY: 'auto', flex: 1, padding: 10 }}>
          {loaded && chain.length === 0 && (
            <div style={{ fontSize: 10, color: hw.textFaint, padding: '6px 2px' }}>
              Nothing in the chain. The instrument hears exactly what is in the clip.
            </div>
          )}

          {chain.map((fx, index) => (
            <div
              key={index}
              style={{
                border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
                padding: 8, marginBottom: 8, background: hw.bgElevated,
              }}
            >
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
                <span style={{ fontSize: 9, color: hw.textFaint, width: 14 }}>{index + 1}</span>
                <span style={{ fontSize: 11, fontWeight: 600 }}>{label(fx)}</span>
                <div style={{ flex: 1 }} />
                <button onClick={() => move(index, -1)} disabled={index === 0} style={btn()}>up</button>
                <button onClick={() => move(index, 1)} disabled={index === chain.length - 1} style={btn()}>down</button>
                <button onClick={() => remove(index)} style={btn()}>remove</button>
              </div>

              {'Arpeggiator' in fx && (
                <div style={{ display: 'flex', gap: 10, alignItems: 'center', flexWrap: 'wrap' }}>
                  <Field label="Step">
                    <select
                      value={fx.Arpeggiator.step_ticks}
                      onChange={(e) => replace(index, {
                        Arpeggiator: { ...fx.Arpeggiator, step_ticks: Number(e.target.value) },
                      })}
                      style={sel()}
                    >
                      {STEP_CHOICES.map(c => (
                        <option key={c.label} value={c.ticks}>{c.label}</option>
                      ))}
                    </select>
                  </Field>
                  <Field label="Order">
                    <select
                      value={fx.Arpeggiator.mode}
                      onChange={(e) => replace(index, {
                        Arpeggiator: { ...fx.Arpeggiator, mode: e.target.value as ArpMode },
                      })}
                      style={sel()}
                    >
                      <option value="Up">Up</option>
                      <option value="Down">Down</option>
                      <option value="UpDown">Up and down</option>
                      <option value="AsPlayed">As written</option>
                    </select>
                  </Field>
                  <Field label="Octaves">
                    <select
                      value={fx.Arpeggiator.octaves}
                      onChange={(e) => replace(index, {
                        Arpeggiator: { ...fx.Arpeggiator, octaves: Number(e.target.value) },
                      })}
                      style={sel()}
                    >
                      {[1, 2, 3, 4].map(o => <option key={o} value={o}>{o}</option>)}
                    </select>
                  </Field>
                  <Field label={`Gate ${Math.round(fx.Arpeggiator.gate * 100)}%`}>
                    <input
                      type="range" min={0.05} max={1} step={0.05}
                      value={fx.Arpeggiator.gate}
                      onChange={(e) => replace(index, {
                        Arpeggiator: { ...fx.Arpeggiator, gate: Number(e.target.value) },
                      })}
                      style={{ width: 110, accentColor: hw.accent }}
                    />
                  </Field>
                </div>
              )}

              {'Chord' in fx && (
                <div style={{ display: 'flex', gap: 10, alignItems: 'center', flexWrap: 'wrap' }}>
                  <Field label="Shape">
                    <select
                      value={fx.Chord.intervals.join(',')}
                      onChange={(e) => replace(index, {
                        Chord: { intervals: e.target.value.split(',').map(Number).filter(n => !Number.isNaN(n)) },
                      })}
                      style={sel()}
                    >
                      {CHORD_CHOICES.map(c => (
                        <option key={c.label} value={c.intervals.join(',')}>{c.label}</option>
                      ))}
                      {!CHORD_CHOICES.some(c => c.intervals.join(',') === fx.Chord.intervals.join(',')) && (
                        <option value={fx.Chord.intervals.join(',')}>
                          Custom ({fx.Chord.intervals.join(', ')})
                        </option>
                      )}
                    </select>
                  </Field>
                  <span style={{ fontSize: 9, color: hw.textFaint }}>
                    Semitones above the note you played: {fx.Chord.intervals.join(', ') || 'none'}
                  </span>
                </div>
              )}

              {'Scale' in fx && (
                <div style={{ display: 'flex', gap: 10, alignItems: 'center', flexWrap: 'wrap' }}>
                  <Field label="Root">
                    <select
                      value={fx.Scale.root}
                      onChange={(e) => replace(index, {
                        Scale: { ...fx.Scale, root: Number(e.target.value) },
                      })}
                      style={sel()}
                    >
                      {NOTE_NAMES.map((n, i) => <option key={n} value={i}>{n}</option>)}
                    </select>
                  </Field>
                  <Field label="Scale">
                    <select
                      value={fx.Scale.kind}
                      onChange={(e) => replace(index, {
                        Scale: { ...fx.Scale, kind: e.target.value as ScaleKind },
                      })}
                      style={sel()}
                    >
                      {(Object.keys(SCALE_LABELS) as ScaleKind[]).map(k => (
                        <option key={k} value={k}>{SCALE_LABELS[k]}</option>
                      ))}
                    </select>
                  </Field>
                  <span style={{ fontSize: 9, color: hw.textFaint }}>
                    A note outside the scale moves to the nearest one that belongs.
                  </span>
                </div>
              )}

              {'Transpose' in fx && (
                <div style={{ display: 'flex', gap: 10, alignItems: 'center' }}>
                  <Field label="Semitones">
                    <input
                      type="number" min={-48} max={48}
                      value={fx.Transpose.semitones}
                      onChange={(e) => replace(index, {
                        Transpose: { semitones: Number(e.target.value) },
                      })}
                      style={{ ...sel(), width: 70 }}
                    />
                  </Field>
                  <span style={{ fontSize: 9, color: hw.textFaint }}>
                    A note pushed past the end of the keyboard is dropped, not folded back.
                  </span>
                </div>
              )}
            </div>
          ))}
        </div>

        <div style={{
          padding: '8px 12px', display: 'flex', gap: 8, alignItems: 'center',
          background: hw.bgElevated, borderTop: `1px solid ${hw.border}`, flexWrap: 'wrap',
        }}>
          <span style={{ fontSize: 10, color: hw.textSecondary }}>Add:</span>
          <button onClick={() => add('arp')} style={btn(true)}>Arpeggiator</button>
          <button onClick={() => add('chord')} style={btn(true)}>Chord</button>
          <button onClick={() => add('scale')} style={btn(true)}>Scale</button>
          <button onClick={() => add('transpose')} style={btn(true)}>Transpose</button>
          <div style={{ flex: 1 }} />
          <span style={{ fontSize: 9, color: hw.textFaint }}>
            Live playing goes through chord, scale and transpose. The arpeggiator needs a clock of its own, so it works on what is written in a clip.
          </span>
        </div>
      </div>
    </div>
  )
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label style={{ display: 'flex', alignItems: 'center', gap: 5, fontSize: 10, color: hw.textSecondary }}>
      {label}
      {children}
    </label>
  )
}

function btn(active: boolean = false) {
  return {
    padding: '3px 10px', fontSize: 10, background: 'transparent',
    border: `1px solid ${active ? hw.accent : hw.border}`, borderRadius: hw.radius.sm,
    color: active ? hw.accent : hw.textSecondary, cursor: 'pointer',
  } as const
}

function sel() {
  return {
    fontSize: 10, padding: '2px 6px', background: hw.bg,
    border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
    color: hw.textPrimary,
  } as const
}
