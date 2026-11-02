/**
 * The sample window's NOTES view: the take as notes on a piano grid, as the
 * founder approved it from the mockup (daw-tune-notes-mockup, 2 November
 * 2026). Each blob is one sung note, where it was sung (its height) and for
 * as long as it lasted; the thin line through it is the pitch as sung,
 * wobble and all. Notes in tune are grey, those more than 20 cents out
 * amber, the chosen ones red.
 *
 * Drag a note up or down: it snaps to the key (or every semitone, or free
 * with Alt). Drag its edges to retime it. Alt+click splits it, double-click
 * puts it on the nearest note of the key. Each move is one undo step, heard
 * at once; the file is never changed (notesModel.ts, tune_notes.rs).
 */
import { useEffect, useMemo, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { invokeOrNull } from '../../api/invoke'
import { useTrackStore, type ClipInfo } from '../../stores/trackStore'
import { useHistoryStore } from '../../stores/historyStore'
import { useProjectStore } from '../../stores/projectStore'
import { FlatCheck, FlatSelect } from '../ui/flat/Flat'
import { t, tf, useLanguage } from '../../i18n'
import {
  KEY_LABELS, OUT_OF_TUNE_CENTS, centsOff, changesSound, describePitch, inKey, keyFromLabel, keyLabel,
  movePitch, nearestInKey, noteName, pitchRange, playedLine, retime, snapPitch, splitNote, tuneAll,
  type ClipNotes, type Key, type Snap, type TuneNote,
} from './notesModel'

const KEYS_W = 44
const RULER = 18
const LANE_H = 236
const EDGE_PX = 6
const SNAPS = ['scale', 'semitone', 'off'] as const
const SNAP_LABEL: Record<Snap, string> = { scale: 'Snap: scale', semitone: 'Snap: semitone', off: 'Snap: off' }
const STRENGTHS = ['100', '90', '75', '50', '25'] as const

/** What a drag is doing, while it lasts. */
type Drag =
  | { kind: 'pitch'; index: number; y0: number; from: TuneNote[]; moved: boolean }
  | { kind: 'start' | 'end'; index: number; x0: number; from: TuneNote[]; moved: boolean }

/** The switch between the waveform and the notes, over the bottom view. */
export function ViewSwitch({ notes, onPick, children }: { notes: boolean; onPick: (notes: boolean) => void; children?: React.ReactNode }) {
  useLanguage()
  return (
    <div className="sw-viewbar">
      <span className="hwf-lab">{t('Sample')}</span>
      <div className="sw-seg" role="tablist">
        <button type="button" role="tab" aria-selected={!notes} className={notes ? '' : 'is-on'} onClick={() => onPick(false)}>{t('WAVE')}</button>
        <button
          type="button" role="tab" aria-selected={notes} className={notes ? 'is-on' : ''} onClick={() => onPick(true)}
          title={t('The take as notes, to tune by hand')}
        >{t('NOTES')}</button>
      </div>
      {children}
    </div>
  )
}

export function NotesView({ trackId, clip, bpm, onWave }: { trackId: string; clip: ClipInfo; bpm: number; onWave: () => void }) {
  useLanguage()
  const [view, setView] = useState<ClipNotes | null>(null)
  const [notes, setNotes] = useState<TuneNote[]>([])
  const [progress, setProgress] = useState(0)
  const [failed, setFailed] = useState<string | null>(null)
  const [chosen, setChosen] = useState<Set<number>>(new Set())
  const [key, setKey] = useState<Key>(null)
  const [snap, setSnap] = useState<Snap>('scale')
  const [strength, setStrength] = useState<(typeof STRENGTHS)[number]>('100')
  const [drag, setDrag] = useState<Drag | null>(null)
  const [hint, setHint] = useState<{ index: number; text: string } | null>(null)
  const keyGuessed = useRef(false)
  const scroller = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(660)

  // The notes: found the first time, read again when an undo changes them.
  const tuneKey = clip.tuneKey ?? ''
  useEffect(() => {
    let live = true
    setFailed(null)
    const stop = listen<{ sourceId: string; done: number }>('tune-notes-progress', (e) => {
      if (live && e.payload.sourceId === clip.source_id) setProgress(e.payload.done)
    })
    invoke<ClipNotes>('find_clip_notes', { trackId, clipId: clip.id })
      .then((v) => {
        if (!live) return
        setView(v)
        setNotes(v.notes)
        setChosen((c) => new Set([...c].filter((i) => i < v.notes.length)))
        if (!keyGuessed.current && v.keyRoot !== null) {
          keyGuessed.current = true
          setKey({ root: v.keyRoot, minor: v.keyMinor })
        }
      })
      .catch((e) => { if (live) setFailed(String(e)) })
    return () => { live = false; void stop.then((un) => un()) }
  }, [trackId, clip.id, clip.source_id, tuneKey])

  useEffect(() => {
    const el = scroller.current
    if (!el) return
    const measure = () => setWidth(Math.max(200, el.clientWidth - KEYS_W))
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [view !== null]) // eslint-disable-line react-hooks/exhaustive-deps

  const seconds = view?.seconds ?? 0
  const beat = 60 / Math.max(1, bpm)
  const pxPerSec = seconds > 0 ? Math.max(width / seconds, 48 / beat) : 100
  const laneW = Math.max(width, seconds * pxPerSec)
  const { low, high } = useMemo(() => pitchRange(notes), [notes])
  const rows = high - low + 1
  const rowH = Math.max(8, (LANE_H - RULER) / rows)
  const laneH = RULER + rows * rowH
  const X = (s: number) => s * pxPerSec
  const Y = (p: number) => RULER + (high - p) * rowH + rowH / 2

  /** Keep the notes and name the step. */
  const commit = async (next: TuneNote[], label: string) => {
    setNotes(next)
    const since = Date.now()
    const changed = await invokeOrNull<number>('set_clip_notes', { trackId, clipId: clip.id, notes: next }, { message: t('Tuning the note failed'), retry: false })
    if (changed === null) {
      // Refused: back to what the engine has.
      setNotes(view?.notes ?? [])
      return
    }
    useProjectStore.getState().markDirty()
    useHistoryStore.getState().push(label, since)
    await useTrackStore.getState().fetchTracks()
  }

  const onNoteDown = (e: React.PointerEvent, index: number) => {
    if (!view || e.button !== 0) return
    e.stopPropagation()
    const n = notes[index]
    const rect = (e.currentTarget as SVGGElement).ownerSVGElement!.getBoundingClientRect()
    const x = e.clientX - rect.left
    if (e.altKey) {
      const next = splitNote(notes, index, x / pxPerSec, view)
      if (next !== notes) {
        setChosen(new Set([index, index + 1]))
        void commit(next, 'Split note')
      }
      return
    }
    const nextChosen = e.shiftKey
      ? new Set(chosen.has(index) ? [...chosen].filter((i) => i !== index) : [...chosen, index])
      : chosen.has(index) ? chosen : new Set([index])
    setChosen(nextChosen)
    const left = X(n.newStart)
    const right = X(n.newEnd)
    const edge = right - left > EDGE_PX * 3 ? (x - left < EDGE_PX ? 'start' : right - x < EDGE_PX ? 'end' : null) : null
    try { (e.currentTarget as Element).setPointerCapture(e.pointerId) } catch { /* not a live pointer */ }
    setDrag(edge
      ? { kind: edge, index, x0: e.clientX, from: notes, moved: false }
      : { kind: 'pitch', index, y0: e.clientY, from: notes, moved: false })
  }

  const onMove = (e: React.PointerEvent) => {
    if (!drag || !view) return
    if (drag.kind === 'pitch') {
      const grabbed = drag.from[drag.index]
      const raw = grabbed.pitch + (drag.y0 - e.clientY) / rowH
      const landed = snapPitch(raw, snap, key, e.altKey)
      const by = landed - grabbed.pitch
      const moving = chosen.has(drag.index) ? chosen : new Set([drag.index])
      setNotes(movePitch(drag.from, moving, by))
      if (Math.abs(by) > 0.001) setDrag({ ...drag, moved: true })
      const now = Math.max(0, Math.min(127, landed))
      const cents = Math.round((now - grabbed.sung) * 100)
      setHint({
        index: drag.index,
        text: `${noteName(now)} · ${cents > 0 ? '+' : ''}${cents} cents · ${inKey(now, key) && centsOff(now) === 0 ? t('in key') : t('out of key')}`,
      })
    } else {
      const n = drag.from[drag.index]
      const to = (drag.kind === 'start' ? n.newStart : n.newEnd) + (e.clientX - drag.x0) / pxPerSec
      setNotes(retime(drag.from, drag.index, drag.kind, to, seconds))
      if (Math.abs(e.clientX - drag.x0) > 1) setDrag({ ...drag, moved: true })
    }
  }

  const onUp = () => {
    if (!drag) return
    const { moved, kind } = drag
    setDrag(null)
    setHint(null)
    if (moved) void commit(notes, kind === 'pitch' ? (chosen.size > 1 ? 'Move notes' : 'Move note') : 'Retime note')
    else setNotes(drag.from)
  }

  const onNoteDouble = (index: number) => {
    const n = notes[index]
    const target = nearestInKey(n.pitch, key)
    if (Math.abs(target - n.pitch) < 0.001) return
    void commit(notes.map((m, i) => (i === index ? { ...m, pitch: target } : m)), 'Tune note')
  }

  const chosenList = [...chosen].filter((i) => i < notes.length).sort((a, b) => a - b)
  const setForChosen = (patch: Partial<TuneNote>, label: string) => {
    if (!chosenList.length) return
    void commit(notes.map((n, i) => (chosen.has(i) ? { ...n, ...patch } : n)), label)
  }

  // The grid: rows in the key lighter, the song's beats and bars.
  const grid = useMemo(() => {
    const parts: React.ReactNode[] = []
    for (let p = high; p >= low; p--) {
      const y = RULER + (high - p) * rowH
      parts.push(<rect key={`r${p}`} x={0} y={y} width={laneW} height={rowH} fill={inKey(p, key) ? 'rgba(255,255,255,.025)' : 'rgba(0,0,0,.25)'} />)
      parts.push(<line key={`l${p}`} x1={0} y1={y} x2={laneW} y2={y} stroke="rgba(255,255,255,.05)" />)
    }
    const barPx = 4 * beat * pxPerSec
    const every = Math.max(1, Math.pow(2, Math.ceil(Math.log2(Math.max(1, 26 / Math.max(1, barPx))))))
    for (let b = 0; b * beat <= seconds + 1e-9; b++) {
      const x = X(b * beat)
      const bar = b % 4 === 0
      parts.push(<line key={`b${b}`} x1={x} y1={bar ? 0 : RULER} x2={x} y2={laneH} stroke={`rgba(255,255,255,${bar ? 0.1 : 0.04})`} />)
      if (bar && (b / 4) % every === 0) parts.push(<text key={`t${b}`} x={x + 4} y={13} fontSize={10.5} fill="#d4d4d8">{b / 4 + 1}</text>)
    }
    return parts
  }, [high, low, rowH, laneW, key, beat, pxPerSec, seconds, laneH]) // eslint-disable-line react-hooks/exhaustive-deps

  const keys = []
  for (let p = high; p >= low; p--) {
    const black = [1, 3, 6, 8, 10].includes(((p % 12) + 12) % 12)
    const y = RULER + (high - p) * rowH
    keys.push(<rect key={`k${p}`} x={0} y={y} width={KEYS_W} height={rowH} fill={black ? '#141418' : '#1d1d23'} stroke="rgba(255,255,255,.06)" />)
    if (rowH >= 9) keys.push(<text key={`n${p}`} x={5} y={y + rowH / 2 + 3.5} fontSize={Math.min(10, rowH - 1)} fill={black ? '#71717a' : '#d4d4d8'}>{noteName(p)}</text>)
  }

  const blob = (i: number, at: TuneNote, look: 'plain' | 'out' | 'chosen' | 'was') => {
    const x = X(at.newStart)
    const w = Math.max(4, X(at.newEnd) - x)
    const h = Math.max(7, Math.min(18, rowH * 0.85))
    const yc = Y(at.pitch)
    const stroke = look === 'chosen' ? '#e5243b' : look === 'out' || look === 'was' ? '#f59e0b' : '#a1a1aa'
    const fill = look === 'chosen' ? 'rgba(229,36,59,.25)' : look === 'out' ? 'rgba(245,158,11,.14)' : look === 'was' ? 'rgba(245,158,11,.06)' : 'rgba(161,161,170,.12)'
    const line = view ? playedLine(at, view) : []
    const d = line.map(([s, p], k) => `${k ? 'L' : 'M'}${X(s).toFixed(1)} ${Y(p).toFixed(1)}`).join('')
    return (
      <g key={`${look}${i}`} pointerEvents={look === 'was' ? 'none' : undefined}>
        <rect
          x={x + 1} y={yc - h / 2} width={Math.max(2, w - 2)} height={h} rx={h / 2}
          fill={fill} stroke={stroke} strokeWidth={look === 'chosen' ? 2 : 1} strokeDasharray={look === 'was' ? '4 3' : undefined}
        />
        {d && <path d={d} fill="none" stroke={stroke} strokeOpacity={0.9} strokeWidth={1.3} />}
      </g>
    )
  }

  const one = chosenList.length >= 1 ? notes[chosenList[chosenList.length - 1]] : null
  const driftShown = one ? Math.round(one.drift * 100) : 100
  const beatOf = (s: number) => Math.floor(s / beat + 1e-6)

  return (
    <div className="sw-notes">
      <ViewSwitch notes onPick={(n) => { if (!n) onWave() }}>
        <FlatSelect value={keyLabel(key)} options={KEY_LABELS} label={(k) => tf('Key: {key}', { key: k === 'Chromatic' ? t('Chromatic') : k.replace(/ (major|minor)$/, (_m, mode: string) => ` ${t(mode)}`) })} onPick={(k) => setKey(keyFromLabel(k))} style={{ width: 132 }} />
        <FlatSelect value={snap} options={SNAPS} label={(s) => t(SNAP_LABEL[s])} onPick={setSnap} style={{ width: 132 }} />
        <FlatSelect value={strength} options={STRENGTHS} label={(s) => tf('Strength {n}%', { n: s })} onPick={setStrength} style={{ width: 136 }} />
        <span className="sw-grow" />
        <button
          type="button" className="sw-tuneall" disabled={!notes.length}
          title={t('Moves every note towards the nearest note of the key; the notes can be moved after')}
          onClick={() => void commit(tuneAll(notes, key, Number(strength) / 100), 'Tune all notes')}
        >{t('Tune all')}</button>
      </ViewSwitch>

      <div
        className="sw-notes-scroll" ref={scroller}
        onPointerDown={() => { if (!drag) setChosen(new Set()) }}
      >
        {!view ? (
          <div className="sw-notes-wait">
            {failed ? <span>{failed}</span> : (
              <>
                <span>{t('Finding the notes…')}</span>
                <div className="sw-progress"><i style={{ width: `${Math.round(progress * 100)}%` }} /></div>
              </>
            )}
          </div>
        ) : (
          <div style={{ display: 'flex', width: KEYS_W + laneW }}>
            <svg className="sw-notes-keys" width={KEYS_W} height={laneH}>
              <rect width={KEYS_W} height={RULER} fill="#0d0d10" />
              {keys}
            </svg>
            <svg
              width={laneW} height={laneH} style={{ display: 'block', fontFamily: 'Inter, sans-serif' }} data-row-height={rowH}
              onPointerMove={onMove} onPointerUp={onUp} onPointerCancel={onUp}
            >
              {grid}
              {!notes.length && <text x={12} y={RULER + 24} fontSize={12} fill="#74747e">{t('No sung notes found in this take')}</text>}
              {drag?.kind === 'pitch' && drag.moved && [...(chosen.has(drag.index) ? chosen : new Set([drag.index]))].map((i) => drag.from[i] && blob(i, drag.from[i], 'was'))}
              {notes.map((n, i) => {
                const look = chosen.has(i) ? 'chosen' : Math.abs(centsOff(n.pitch)) > OUT_OF_TUNE_CENTS ? 'out' : 'plain'
                const x = X(n.newStart)
                const c = centsOff(n.pitch)
                return (
                  <g
                    key={i} style={{ cursor: 'grab' }} data-note={i}
                    onPointerDown={(e) => onNoteDown(e, i)}
                    onDoubleClick={(e) => { e.stopPropagation(); onNoteDouble(i) }}
                  >
                    {blob(i, n, look)}
                    {X(n.newEnd) - x > 34 && (
                      <text x={x + 6} y={Y(n.pitch) + Math.max(7, Math.min(18, rowH * 0.85)) / 2 + 11} fontSize={10} fill={look === 'chosen' ? '#fda4af' : '#a1a1aa'} pointerEvents="none">
                        {noteName(n.pitch)} {c > 0 ? '+' : ''}{c}
                      </text>
                    )}
                    {/* the edges, for retiming */}
                    <rect x={x} y={Y(n.pitch) - 9} width={EDGE_PX} height={18} fill="transparent" style={{ cursor: 'ew-resize' }} />
                    <rect x={X(n.newEnd) - EDGE_PX} y={Y(n.pitch) - 9} width={EDGE_PX} height={18} fill="transparent" style={{ cursor: 'ew-resize' }} />
                  </g>
                )
              })}
              {hint && notes[hint.index] && (() => {
                const n = notes[hint.index]
                const tx = Math.min(laneW - 160, X(n.newEnd) + 8)
                const ty = Math.max(RULER + 2, Y(n.pitch) - 30)
                return (
                  <g pointerEvents="none">
                    <line x1={X((n.newStart + n.newEnd) / 2)} y1={Y(drag?.from[hint.index]?.pitch ?? n.pitch)} x2={X((n.newStart + n.newEnd) / 2)} y2={Y(n.pitch)} stroke="#e5243b" strokeDasharray="2 3" />
                    <rect x={tx} y={ty} width={152} height={20} rx={4} fill="#18181b" stroke="#3f3f46" />
                    <text x={tx + 7} y={ty + 14} fontSize={10.5} fontWeight={600} fill="#fafafa">{hint.text}</text>
                  </g>
                )
              })()}
            </svg>
          </div>
        )}
      </div>

      <div className="sw-notes-side">
        {one ? (
          <>
            <span>{t('Selected:')} <b>{tf('bar {bar} beat {beat}', { bar: Math.floor(beatOf(one.newStart) / 4) + 1, beat: (beatOf(one.newStart) % 4) + 1 })}</b>{chosenList.length > 1 ? ` ${tf('(+{n} more)', { n: chosenList.length - 1 })}` : ''}</span>
            <span>{t('Sung:')} <b>{describePitch(one.sung)}</b></span>
            <span>{t('Now:')} <b>{describePitch(one.pitch)}</b>{changesSound({ ...one, drift: 1, newStart: one.start, newEnd: one.end }) ? ` ${tf('(moved {n})', { n: `${one.pitch >= one.sung ? '+' : ''}${Math.round((one.pitch - one.sung) * 100)}` })}` : ''}</span>
            <label className="sw-drift" title={t('How much of the wobble (vibrato, the slide into the note) is kept; 0% is the hard-tuned sound')}>
              {t('Drift kept:')} <b>{driftShown}%</b>
              <input
                key={`${chosenList.join(',')}:${driftShown}`}
                type="range" min={0} max={100} step={5} defaultValue={driftShown}
                onPointerUp={(e) => setForChosen({ drift: Number((e.target as HTMLInputElement).value) / 100 }, 'Note drift')}
                onKeyUp={(e) => setForChosen({ drift: Number((e.target as HTMLInputElement).value) / 100 }, 'Note drift')}
              />
            </label>
            <span>{t('Length:')} <b>{Math.abs((one.newEnd - one.newStart) - (one.end - one.start)) < 0.0005 && Math.abs(one.newStart - one.start) < 0.0005
              ? t('as sung')
              : tf('{now} s (sung {was} s)', { now: (one.newEnd - one.newStart).toFixed(2), was: (one.end - one.start).toFixed(2) })}</b></span>
            <FlatCheck
              label={t('Formants kept')} on={one.formants}
              hint={t('On, a moved voice keeps its character; off, it moves with the pitch')}
              onToggle={() => setForChosen({ formants: !one.formants }, one.formants ? 'Formants moved' : 'Formants kept')}
            />
          </>
        ) : (
          <span>{view && notes.length
            ? t('Click a note to choose it. Drag it up or down (hold Alt while dragging for no snap), drag its edges to retime it, Alt+click to split it, double-click to put it on the key.')
            : ' '}</span>
        )}
      </div>
    </div>
  )
}
