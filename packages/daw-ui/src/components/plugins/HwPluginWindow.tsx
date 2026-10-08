import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { CSSProperties, PointerEvent as ReactPointerEvent } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { usePluginPresetStore } from '../../stores/pluginPresetStore'
import { useNotificationStore } from '../../stores/notificationStore'
import { useTrackStore } from '../../stores/trackStore'
import {
  EQ_BAND_COLOURS, EQ_BAND_KINDS, FAMILY, type FamilyColours, type Panel, type PluginLayout, type Readout, type Row,
  layoutFor, windowWidth,
} from './pluginLayouts'
import {
  LIVE_KINDS, NO_LIVE, ParamView, SCOPE_KINDS, correlation, drawDisplay, drawKnob, eqBands, freqX, toDb, valueForNumber, xFreq, yEq,
  type LiveData, type PluginParam,
} from './pluginDraw'
import { ParamMenu, type ParamMenuTarget } from './ParamMenu'
import './hwPlugin.css'

export interface HwPluginWindowProps {
  trackId: string
  slotId: string
  pluginId: string
  pluginName: string
  /** In an OS window of its own: fill it and fit the window to the page. */
  own?: boolean
  /** Shown as a × at the end of the header when the window floats in the app. */
  onClose?: () => void
  /** Pointer-down on the header, for a floating window to be dragged by. */
  onHeaderPointerDown?: (e: ReactPointerEvent) => void
}

const ICON: Record<string, string> = {
  dyn: '<path d="M3 16h4l3-9 4 12 3-7h4" stroke="var(--a2)" stroke-width="2" fill="none" stroke-linecap="round" stroke-linejoin="round"/>',
  eq: '<path d="M2 14c3 0 4-6 7-6s3 8 6 8 4-4 7-4" stroke="var(--a2)" stroke-width="2" fill="none" stroke-linecap="round"/>',
  time: '<path d="M4 18V8M9 18v-6M14 18v-4M19 18v-2" stroke="var(--a2)" stroke-width="2.2" stroke-linecap="round"/>',
  mod: '<path d="M2 12c2.5-6 5-6 7.5 0s5 6 7.5 0 3-4 5-2" stroke="var(--a2)" stroke-width="2" fill="none" stroke-linecap="round"/>',
  drive: '<path d="M3 17c4 0 4-10 9-10s5 10 9 10" stroke="var(--a2)" stroke-width="2" fill="none" stroke-linecap="round"/><path d="M3 12h18" stroke="var(--a2)" stroke-opacity=".3"/>',
  space: '<circle cx="12" cy="12" r="8" stroke="var(--a2)" stroke-width="2" fill="none"/><path d="M12 4v16M4 12h16" stroke="var(--a2)" stroke-opacity=".4"/>',
  synth: '<rect x="3" y="5" width="18" height="14" rx="2" stroke="var(--a2)" stroke-width="2" fill="none"/><path d="M8 5v9M12 5v9M16 5v9" stroke="var(--a2)" stroke-width="2"/>',
  fx: '<path d="M4 6h4v12H4zM10 6h4v12h-4zM16 6h4v12h-4z" stroke="var(--a2)" stroke-width="1.8" fill="none"/>',
}

const short = (n: string) => n.replace(/^(Band \d|Osc\d) /, '').replace(/^(Low|Mid|High) (?=Thresh|Ratio)/, '')
const HISTORY = 120 // polls kept for the 4 s displays, at 30 a second
const POLL_MS = 33

/** Every display kind a layout uses, to know what the window must poll. */
function displayKinds(l: PluginLayout): string[] {
  if (l.custom === 'eq') return ['eq']
  const out: string[] = []
  const walk = (p?: Panel) => p?.s.forEach((rows) => rows.forEach((r) => { if (r[0] === 'd') out.push(r[1] as string) }))
  l.panels?.forEach((col) => col.forEach(walk))
  walk(l.top); walk(l.bottom)
  return out
}

/**
 * A built-in plug-in's window in the lineup's design: header with presets,
 * A/B, mix and bypass; the layout's panels; IN and OUT meters; every control
 * with FL's right-click menu (automation clip, lane, MIDI learn, type, reset,
 * copy, paste).
 */
export function HwPluginWindow({ trackId, slotId, pluginId, pluginName, own, onClose, onHeaderPointerDown }: HwPluginWindowProps) {
  const layout = layoutFor(pluginId)
  if (!layout) return null
  return (
    <WindowBody
      layout={layout} trackId={trackId} slotId={slotId} pluginId={pluginId} pluginName={pluginName}
      own={own} onClose={onClose} onHeaderPointerDown={onHeaderPointerDown}
    />
  )
}

function WindowBody({ layout, trackId, slotId, pluginId, pluginName, own, onClose, onHeaderPointerDown }: HwPluginWindowProps & { layout: PluginLayout }) {
  const fam = FAMILY[layout.fam]
  const [params, setParams] = useState<PluginParam[] | null>(null)
  const [values, setValues] = useState<Record<number, number>>({})
  const [exact, setExact] = useState<Record<number, string>>({})
  const [error, setError] = useState<string | null>(null)
  const [generation, setGeneration] = useState(0)
  const [eqBand, setEqBand] = useState(0)
  const [menu, setMenu] = useState<ParamMenuTarget | null>(null)
  const [ab, setAb] = useState<'A' | 'B'>('A')
  const abStore = useRef<{ A: Record<number, number> | null; B: Record<number, number> | null }>({ A: null, B: null })
  const [version, setVersion] = useState('')
  const rootRef = useRef<HTMLDivElement>(null)

  const insert = useTrackStore((s) => s.tracks.find((t) => t.id === trackId)?.inserts?.find((i) => i.id === slotId))
  const track = useTrackStore((s) => s.tracks.find((t) => t.id === trackId))
  const [enabled, setEnabled] = useState(insert?.enabled ?? true)
  useEffect(() => { if (insert) setEnabled(insert.enabled) }, [insert?.enabled])

  // Which of this slot's parameters have automation, to mark them.
  const automated = useMemo(() => {
    const ids = new Set<number>()
    const mark = (t: { kind: string; slotId?: string; paramId?: number }) => {
      if (t.kind === 'plugin_param' && t.slotId === slotId && t.paramId != null) ids.add(t.paramId)
    }
    track?.automationLanes?.forEach((l) => mark(l.target as never))
    track?.automationClips?.forEach((c) => mark(c.target as never))
    return ids
  }, [track?.automationLanes, track?.automationClips, slotId])

  useEffect(() => {
    import('@tauri-apps/api/app').then(({ getVersion }) => getVersion()).then(setVersion).catch(() => setVersion(''))
  }, [])

  // The slot's parameters, with the values it is playing.
  useEffect(() => {
    let cancelled = false
    invoke<PluginParam[]>('get_plugin_parameters', { trackId, slotId })
      .then((list) => {
        if (cancelled) return
        setParams(list)
        const v: Record<number, number> = {}
        const e: Record<number, string> = {}
        list.forEach((q) => { v[q.id] = q.value; if (q.text) e[q.id] = q.text })
        setValues(v); setExact(e); setError(null)
      })
      .catch((err) => { if (!cancelled) setError(String(err)) })
    return () => { cancelled = true }
  }, [trackId, slotId, generation])

  const view = useMemo(() => new ParamView(params ?? [], values, exact), [params, values, exact])

  // Moves go to the engine once a frame, the latest value per parameter.
  const pending = useRef(new Map<number, number>())
  const flushQueued = useRef(false)
  const flush = useCallback(() => {
    flushQueued.current = false
    const batch = [...pending.current]
    pending.current.clear()
    batch.forEach(([paramId, value]) => {
      invoke<string | null>('set_plugin_parameter', { trackId, slotId, paramId, value })
        .then((text) => {
          if (text != null) setExact((e) => ({ ...e, [paramId]: text }))
        })
        .catch((err) => console.error('set_plugin_parameter failed', err))
    })
  }, [trackId, slotId])

  const setParam = useCallback((q: PluginParam, value: number) => {
    const v = Math.max(q.min, Math.min(q.max, value))
    setValues((prev) => (prev[q.id] === v ? prev : { ...prev, [q.id]: v }))
    setExact((e) => {
      if (!(q.id in e)) return e
      const next = { ...e }
      delete next[q.id]
      return next
    })
    pending.current.set(q.id, v)
    if (!flushQueued.current) {
      flushQueued.current = true
      requestAnimationFrame(flush)
    }
  }, [flush])

  // ---- live data from the slot
  const kinds = useMemo(() => displayKinds(layout), [layout])
  const wantsScope = kinds.some((k) => SCOPE_KINDS.has(k))
  const live = useRef<LiveData>({ ...NO_LIVE, grHistory: [], inHistory: [] })
  const [liveTick, setLiveTick] = useState(0)

  useEffect(() => {
    let alive = true
    let busy = false
    let n = 0
    const timer = window.setInterval(() => {
      if (busy || document.hidden) return
      busy = true
      n++
      const scope = wantsScope && n % 2 === 0
      Promise.all([
        invoke<[number, number, number, number]>('get_slot_levels', { trackId, slotId }),
        invoke<{ trackId: string; slotId: string; reductionDb: number }[]>('get_gain_reduction'),
        scope ? invoke<number[]>('get_slot_scope', { trackId, slotId }) : Promise.resolve(null),
      ])
        .then(([levels, reductions, frames]) => {
          if (!alive) return
          const l = live.current
          l.levels = levels
          const gr = reductions.find((r) => r.trackId === trackId && r.slotId === slotId)
          l.gr = gr ? gr.reductionDb : null
          l.grHistory = [...l.grHistory, l.gr ?? 0].slice(-HISTORY)
          l.inHistory = [...l.inHistory, toDb(Math.max(levels[0], levels[1]))].slice(-HISTORY)
          if (frames) l.scope = frames
          setLiveTick((t) => (t + 1) % 1_000_000)
        })
        .catch(() => { /* the slot may be going away */ })
        .finally(() => { busy = false })
    }, POLL_MS)
    return () => { alive = false; window.clearInterval(timer) }
  }, [trackId, slotId, wantsScope])

  // The wavetable's frames, for its display.
  const bank = view.t('Bank')
  useEffect(() => {
    if (!kinds.includes('table') || !bank) return
    invoke<number[][]>('wavetable_frames', { bank, positions: 14, points: 128 })
      .then((frames) => { live.current.table = frames; setLiveTick((t) => t + 1) })
      .catch(() => { live.current.table = null })
  }, [bank, kinds])

  // ---- fit an own window to the page once it has drawn
  useLayoutEffect(() => {
    if (!own || !params) return
    const el = rootRef.current
    if (!el) return
    const h = Math.ceil(el.scrollHeight) + 22 // the panel window's drag bar
    invoke('fit_panel_window', { width: windowWidth(layout), height: h }).catch(() => { /* not a panel window */ })
  }, [own, params, layout])

  // ---- A/B: two sets of settings to compare, as in the lineup
  const switchAb = (to: 'A' | 'B') => {
    if (to === ab || !params) return
    abStore.current[ab] = { ...values }
    const target = abStore.current[to] ?? { ...values }
    abStore.current[to] = target
    params.forEach((q) => { if (target[q.id] != null && target[q.id] !== values[q.id]) setParam(q, target[q.id]) })
    setAb(to)
  }

  const setBypass = (on: boolean) => {
    setEnabled(on)
    invoke('set_insert_enabled', { trackId, slotId, enabled: on }).catch((e) => {
      setEnabled(!on)
      useNotificationStore.getState().push('error', `Could not ${on ? 'turn on' : 'bypass'} ${pluginName}: ${String(e)}`)
    })
  }

  const openMenu = (e: React.MouseEvent, q: PluginParam) => {
    e.preventDefault()
    e.stopPropagation()
    setMenu({ x: e.clientX, y: e.clientY, param: q })
  }

  const ctx: Ctx = {
    view, fam, live: live.current, liveTick, setParam, openMenu, automated, eqBand, setEqBand, kinds,
  }

  const style = {
    '--w': `${windowWidth(layout)}px`, '--a': fam.a, '--a2': fam.a2, '--deep': fam.deep,
  } as CSSProperties

  const name = pluginName.replace(/^Hardwave /, '')
  const lv = live.current.levels

  return (
    <div ref={rootRef} className={`hwp${own ? ' own' : ''}`} style={style} onContextMenu={(e) => e.preventDefault()}>
      <div className="hdr" onPointerDown={onHeaderPointerDown}>
        <div>
          <div className="tile"><svg viewBox="0 0 24 24" dangerouslySetInnerHTML={{ __html: ICON[layout.fam] }} /></div>
          <div className="nm"><b>{name}</b><small>{layout.sub}</small></div>
        </div>
        <div onPointerDown={(e) => e.stopPropagation()}>
          <PresetPicker trackId={trackId} slotId={slotId} pluginId={pluginId} onLoaded={() => setGeneration((g) => g + 1)} />
        </div>
        <div onPointerDown={(e) => e.stopPropagation()}>
          <button className={`ab${ab === 'A' ? ' on' : ''}`} onClick={() => switchAb('A')} title="Settings A">A</button>
          <button className={`ab${ab === 'B' ? ' on' : ''}`} onClick={() => switchAb('B')} title="Settings B, to compare with A">B</button>
        </div>
        {layout.hp && view.par(layout.hp) && (
          <div onPointerDown={(e) => e.stopPropagation()}><HeaderSeg ctx={ctx} name={layout.hp} /></div>
        )}
        <div className="grow" />
        {layout.mix && view.par(layout.mix) && (
          <div className="end" onPointerDown={(e) => e.stopPropagation()}><HeaderMix ctx={ctx} name={layout.mix} /></div>
        )}
        <div className="end" onPointerDown={(e) => e.stopPropagation()}>
          <button className={`byp${enabled ? '' : ' off'}`} onClick={() => setBypass(!enabled)} title={enabled ? 'Bypass' : 'Turn on'}>
            {enabled ? 'ACTIVE' : 'BYPASS'}
          </button>
          {onClose && (
            <button className="arw" onClick={onClose} aria-label="Close" title="Close (Esc)">✕</button>
          )}
        </div>
      </div>

      <div className="main">
        {!layout.outOnly && <Strip label="IN" l={lv[0]} r={lv[1]} />}
        {error ? (
          <div className="note" style={{ flex: 1, padding: 20 }}>Could not read {name}&apos;s settings: {error}</div>
        ) : !params ? (
          <div className="note" style={{ flex: 1, padding: 20 }}>Loading…</div>
        ) : layout.custom === 'eq' ? (
          <EqBody ctx={ctx} />
        ) : (
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 10, minWidth: 0 }}>
            {layout.top && <PanelView ctx={ctx} p={layout.top} />}
            <div className="cols" style={{ '--cols': layout.cols } as CSSProperties}>
              {layout.panels?.map((col, i) => (
                <div className="col" key={i}>{col.map((p, k) => <PanelView ctx={ctx} p={p} key={k} />)}</div>
              ))}
            </div>
            {layout.bottom && <PanelView ctx={ctx} p={layout.bottom} wide />}
          </div>
        )}
        <Strip label="OUT" l={lv[2]} r={lv[3]} />
      </div>

      <div className="ftr">
        <span>Hardwave&nbsp;<span style={{ color: 'var(--sub)' }}>Studios</span></span>
        <span><i className="st" style={enabled ? undefined : { background: 'var(--dim)', boxShadow: 'none' }} />{enabled ? 'Running' : 'Bypassed'}</span>
        <span className="grow" />
        <span className="r">Built in</span>
        {version && <span className="r"><b>v{version}</b></span>}
      </div>

      {menu && (
        <ParamMenu
          target={menu}
          view={view}
          trackId={trackId}
          slotId={slotId}
          pluginName={name}
          accent={fam.a2}
          automated={automated.has(menu.param.id)}
          onSet={(v) => setParam(menu.param, v)}
          onClose={() => setMenu(null)}
        />
      )}
    </div>
  )
}

// ------------------------------------------------------------------ parts

interface Ctx {
  view: ParamView
  fam: FamilyColours
  live: LiveData
  liveTick: number
  setParam: (q: PluginParam, v: number) => void
  openMenu: (e: React.MouseEvent, q: PluginParam) => void
  automated: Set<number>
  eqBand: number
  setEqBand: (i: number) => void
  kinds: string[]
}

function PanelView({ ctx, p, wide }: { ctx: Ctx; p: Panel; wide?: boolean }) {
  const onQ = p.on ? ctx.view.par(p.on) : undefined
  const on = onQ ? ctx.view.norm(onQ) >= 0.5 : true
  return (
    <div className={`pnl${p.grow ? ' grow' : ''}`}>
      <div className="ph">
        <span className="dot" />
        <span className="t">{p.t}</span>
        {p.x && <span className="x">{p.x}</span>}
        {onQ && (
          <span
            className={`tg${on ? ' on' : ''}`}
            onClick={() => ctx.setParam(onQ, on ? onQ.min : onQ.max)}
            onContextMenu={(e) => ctx.openMenu(e, onQ)}
            title={`${onQ.name}: ${ctx.view.text(onQ)}`}
          >
            {short(onQ.name).toUpperCase()}<i />
          </span>
        )}
      </div>
      {p.s.map((rows, i) => (
        <div key={i} className={`sec${i === p.s.length - 1 && p.grow ? ' fill' : ''}`}>
          {wide ? (
            <div style={{ display: 'grid', gridTemplateColumns: '1.1fr 1fr', gap: 16, alignItems: 'center' }}>
              {rows.map((r, k) => <RowView ctx={ctx} r={r} key={k} />)}
            </div>
          ) : (
            <div className="rows">{rows.map((r, k) => <RowView ctx={ctx} r={r} key={k} />)}</div>
          )}
        </div>
      ))}
    </div>
  )
}

function RowView({ ctx, r }: { ctx: Ctx; r: Row }) {
  const kind = r[0]
  if (kind === 'k') return <div className="knobs">{(r.slice(1) as string[]).map((n) => <KnobFor ctx={ctx} name={n} key={n} />)}</div>
  if (kind === 'K') return <div className="knobs"><KnobFor ctx={ctx} name={r[1] as string} big /></div>
  if (kind === 'p') return <Pills ctx={ctx} name={r[1] as string} />
  if (kind === 'w') return <SwitchRow ctx={ctx} name={r[1] as string} />
  if (kind === 'd') return <Display ctx={ctx} kind={r[1] as string} h={r[2] as number} />
  if (kind === 'r') return <Readouts ctx={ctx} items={r.slice(1) as Readout[]} />
  if (kind === 'n') return <div className="note">{r[1] as string}</div>
  if (kind === 'm') return <LevelRows live={ctx.live} />
  if (kind === 'g') return <GrRow live={ctx.live} />
  if (kind === 'load') return <LoadSample />
  return null
}

function KnobFor({ ctx, name, big }: { ctx: Ctx; name: string; big?: boolean }) {
  const q = ctx.view.par(name)
  if (!q) return null
  // A parameter that picks one of a few reads better as pills.
  return (
    <Knob
      q={q} big={big} t={ctx.view.norm(q)} text={ctx.view.text(q)} col={ctx.fam}
      auto={ctx.automated.has(q.id)}
      onSet={(v) => ctx.setParam(q, v)} denorm={(t) => ctx.view.denorm(q, t)}
      onMenu={(e) => ctx.openMenu(e, q)}
    />
  )
}

const Knob = memo(function Knob({ q, big, t, text, col, auto, onSet, denorm, onMenu }: {
  q: PluginParam; big?: boolean; t: number; text: string; col: FamilyColours; auto: boolean
  onSet: (v: number) => void; denorm: (t: number) => number; onMenu: (e: React.MouseEvent) => void
}) {
  const size = big ? 64 : 46
  const ref = useRef<HTMLCanvasElement>(null)
  useEffect(() => { if (ref.current) drawKnob(ref.current, t, col) }, [t, col])
  const onPointerDown = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    if (e.button !== 0) return
    const c = e.currentTarget
    c.setPointerCapture(e.pointerId)
    const y0 = e.clientY, s0 = t
    const move = (ev: PointerEvent) => {
      // Shift for fine moves, as in the lineup.
      const per = ev.shiftKey ? 800 : 200
      onSet(denorm(Math.max(0, Math.min(1, s0 + (y0 - ev.clientY) / per))))
    }
    const up = () => { c.removeEventListener('pointermove', move); c.removeEventListener('pointerup', up) }
    c.addEventListener('pointermove', move); c.addEventListener('pointerup', up)
  }
  return (
    <div className={`kb${big ? ' big' : ''}${auto ? ' auto' : ''}`} onContextMenu={onMenu} title={`${q.name}: ${text} · right-click for automation`}>
      <span className="lbl">{short(q.name)}</span>
      <canvas
        ref={ref} width={size * 2} height={size * 2} style={{ width: size, height: size }}
        onPointerDown={onPointerDown}
        onDoubleClick={() => onSet(q.defaultValue)}
        onWheel={(e) => { onSet(denorm(Math.max(0, Math.min(1, t + (e.deltaY < 0 ? 1 : -1) * (e.shiftKey ? 0.002 : 0.01))))) }}
      />
      <span className="v">{text}</span>
    </div>
  )
})

function choicesOf(view: ParamView, q: PluginParam): string[] | null {
  if (q.options && q.options.length > 1) return q.options
  if (view.isSwitch(q)) return ['Off', 'On']
  return null
}

function Pills({ ctx, name }: { ctx: Ctx; name: string }) {
  const q = ctx.view.par(name)
  if (!q) return null
  const opts = choicesOf(ctx.view, q)
  if (!opts) return <div className="knobs"><KnobFor ctx={ctx} name={name} /></div>
  const i = Math.round(ctx.view.norm(q) * (opts.length - 1))
  return (
    <div className={`pills${ctx.automated.has(q.id) ? ' auto' : ''}`} onContextMenu={(e) => ctx.openMenu(e, q)}>
      <span className="lbl">{short(name)}</span>
      {opts.map((o, k) => (
        <button key={o} className={k === i ? 'on' : ''} onClick={() => ctx.setParam(q, ctx.view.denorm(q, opts.length > 1 ? k / (opts.length - 1) : 0))}>{o}</button>
      ))}
    </div>
  )
}

function HeaderSeg({ ctx, name }: { ctx: Ctx; name: string }) {
  const q = ctx.view.par(name)
  if (!q?.options) return null
  const i = ctx.view.choice(name)
  return (
    <>
      <span className="lbl">{short(name)}</span>
      <div className="seg pills" onContextMenu={(e) => ctx.openMenu(e, q)}>
        {q.options.map((o, k) => (
          <button key={o} className={k === i ? 'on' : ''} onClick={() => ctx.setParam(q, ctx.view.denorm(q, k / (q.options!.length - 1)))}>{o}</button>
        ))}
      </div>
    </>
  )
}

function HeaderMix({ ctx, name }: { ctx: Ctx; name: string }) {
  const q = ctx.view.par(name)!
  const t = ctx.view.norm(q)
  const trk = useRef<HTMLDivElement>(null)
  const setFrom = (x: number) => {
    const r = trk.current?.getBoundingClientRect()
    if (r) ctx.setParam(q, ctx.view.denorm(q, (x - r.left) / r.width))
  }
  return (
    <div className="hmix" onContextMenu={(e) => ctx.openMenu(e, q)}>
      <span className="lbl">Mix</span>
      <div
        className="trk" ref={trk}
        onPointerDown={(e) => {
          const el = e.currentTarget
          el.setPointerCapture(e.pointerId)
          setFrom(e.clientX)
          const move = (ev: PointerEvent) => setFrom(ev.clientX)
          const up = () => { el.removeEventListener('pointermove', move); el.removeEventListener('pointerup', up) }
          el.addEventListener('pointermove', move); el.addEventListener('pointerup', up)
        }}
        onDoubleClick={() => ctx.setParam(q, q.defaultValue)}
      >
        <b style={{ width: `${t * 100}%` }} />
      </div>
      <span>{ctx.view.text(q)}</span>
    </div>
  )
}

function SwitchRow({ ctx, name }: { ctx: Ctx; name: string }) {
  const q = ctx.view.par(name)
  if (!q) return null
  const on = ctx.view.norm(q) >= 0.5
  return (
    <div className={`sw${ctx.automated.has(q.id) ? ' auto' : ''}`} onContextMenu={(e) => ctx.openMenu(e, q)}>
      <span className="lbl">{short(name)}</span>
      <button className={`btn${on ? ' on' : ''}`} onClick={() => ctx.setParam(q, on ? q.min : q.max)}>{on ? 'ON' : 'OFF'}</button>
    </div>
  )
}

function Display({ ctx, kind, h }: { ctx: Ctx; kind: string; h: number }) {
  const ref = useRef<HTMLCanvasElement>(null)
  const [corner, setCorner] = useState('')
  const isLive = LIVE_KINDS.has(kind) || kind === 'table'
  useEffect(() => {
    if (!ref.current) return
    setCorner(drawDisplay(ref.current, kind, ctx.view, ctx.fam, ctx.live, { eqBand: ctx.eqBand, time: performance.now() }))
  }, [kind, ctx.view, ctx.fam, ctx.eqBand, isLive ? ctx.liveTick : 0])
  // Redraw when the window changes size, with what is current then.
  const latest = useRef(ctx)
  latest.current = ctx
  useEffect(() => {
    const c = ref.current
    if (!c || typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(() => {
      const x = latest.current
      setCorner(drawDisplay(c, kind, x.view, x.fam, x.live, { eqBand: x.eqBand }))
    })
    ro.observe(c)
    return () => ro.disconnect()
  }, [kind])

  const label = kind.startsWith('wave:') ? 'WAVE' : kind === 'gonio' ? 'STEREO FIELD' : kind.toUpperCase()
  const drag = kind === 'xover' ? xoverDrag(ctx, ref) : undefined
  return (
    <div className={`disp${drag ? ' drag' : ''}`} style={{ height: h }}>
      <canvas ref={ref} onPointerDown={drag} />
      <span className="dl">{label}</span>
      <span className="dr">{corner}</span>
    </div>
  )
}

/** Drag the multiband's crossovers on its display. */
function xoverDrag(ctx: Ctx, ref: React.RefObject<HTMLCanvasElement | null>) {
  return (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const c = ref.current
    if (!c) return
    const r = c.getBoundingClientRect()
    const lo = ctx.view.par('Low/Mid'), hi = ctx.view.par('Mid/High')
    if (!lo || !hi) return
    const x = e.clientX - r.left
    const dLo = Math.abs(x - freqX(ctx.view.num('Low/Mid', 200), r.width))
    const dHi = Math.abs(x - freqX(ctx.view.num('Mid/High', 2000), r.width))
    const q = dLo <= dHi ? lo : hi
    c.setPointerCapture(e.pointerId)
    const move = (ev: PointerEvent) => {
      const hz = xFreq(ev.clientX - r.left, r.width)
      ctx.setParam(q, valueForNumber(ctx.view, q, hz))
    }
    const up = () => { c.removeEventListener('pointermove', move); c.removeEventListener('pointerup', up) }
    c.addEventListener('pointermove', move); c.addEventListener('pointerup', up)
    move(e.nativeEvent)
  }
}

function Readouts({ ctx, items }: { ctx: Ctx; items: Readout[] }) {
  const l = ctx.live
  const fmtDb = (lin: number) => { const d = toDb(lin); return d > -100 ? `${d.toFixed(1)} dB` : '-inf' }
  const cell = (r: Readout): [string, string] => {
    if (r === 'in') return ['Peak in', fmtDb(Math.max(l.levels[0], l.levels[1]))]
    if (r === 'out') return ['Peak out', fmtDb(Math.max(l.levels[2], l.levels[3]))]
    if (r === 'gr') return ['Reduction', l.gr == null ? '-' : `${l.gr.toFixed(1)} dB`]
    if (r === 'grmax') return ['Max GR', `${l.grHistory.reduce((m, x) => Math.min(m, x), 0).toFixed(1)} dB`]
    if (r === 'gate') return ['Gate', l.gr == null ? '-' : l.gr > -1 ? 'Open' : 'Closed']
    if (r === 'corr') return ['Correlation', l.scope && l.scope.some((x) => x !== 0) ? correlation(l.scope).toFixed(2) : '-']
    if (r === 'repeats' || r === 'tail') {
      const fb = Math.min(1, ctx.view.num('Feedback', 0) / 100)
      // Echoes until they are 60 dB down.
      const repeats = fb <= 0 ? 1 : fb >= 1 ? Infinity : Math.ceil(Math.log(0.001) / Math.log(fb))
      if (r === 'repeats') return ['Repeats', Number.isFinite(repeats) ? String(repeats) : '∞']
      const ms = repeats * ctx.view.num('Time', 0)
      return ['Tail', !Number.isFinite(ms) ? '∞' : ms >= 1000 ? `${(ms / 1000).toFixed(1)} s` : `${Math.round(ms)} ms`]
    }
    const name = r.slice(6)
    return [name, ctx.view.t(name)]
  }
  return (
    <div className="ro">
      {items.map((r) => { const [k, v] = cell(r); return <div key={r}><small>{k}</small><b>{v}</b></div> })}
    </div>
  )
}

const pct = (lin: number) => `${Math.max(0, Math.min(1, 1 + toDb(lin) / 60)) * 100}%`

function LevelRows({ live }: { live: LiveData }) {
  const [il, ir, ol, or] = live.levels
  const row = (label: string, a: number, b: number) => (
    <>
      <div className="row"><span style={{ minWidth: 34 }}>{label}</span><i><b style={{ width: pct(a) }} /></i><span className="db">{toDb(Math.max(a, b)) > -100 ? toDb(Math.max(a, b)).toFixed(1) : '-inf'}</span></div>
      <div className="row"><span style={{ minWidth: 34 }} /><i><b style={{ width: pct(b) }} /></i><span className="db" /></div>
    </>
  )
  return <div className="hm">{row('IN', il, ir)}{row('OUT', ol, or)}</div>
}

function GrRow({ live }: { live: LiveData }) {
  const gr = live.gr ?? 0
  return (
    <div className="hm">
      <div className="row">
        <span style={{ minWidth: 34 }}>GR</span>
        <i><b className="gr" style={{ width: `${Math.min(1, -gr / 24) * 100}%` }} /></i>
        <span className="db">{live.gr == null ? '-' : gr.toFixed(1)}</span>
      </div>
    </div>
  )
}

function Strip({ label, l, r }: { label: string; l: number; r: number }) {
  const h = (x: number) => `${Math.max(0, Math.min(1, 1 + toDb(x) / 60)) * 100}%`
  return (
    <div className="strip">
      <span className="lbl">{label}</span>
      <div className="bars"><i><b style={{ height: h(l) }} /></i><i><b style={{ height: h(r) }} /></i></div>
      <em>-60</em>
    </div>
  )
}

function LoadSample() {
  return (
    <div className="note">
      With this channel selected, right-click a sample in the browser and choose <b style={{ color: 'var(--txt)' }}>Send to selected channel</b>.
    </div>
  )
}

// ----------------------------------------------------------------- the EQ

function EqBody({ ctx }: { ctx: Ctx }) {
  const v = ctx.view
  const bands = eqBands(v)
  const b = ctx.eqBand + 1
  const col = EQ_BAND_COLOURS[ctx.eqBand] ?? '#2DD4BF'
  const onQ = v.par(`Band ${b} Enabled`)
  const kindName = { lowshelf: 'Low shelf', highshelf: 'High shelf', peak: 'Bell' }[EQ_BAND_KINDS[ctx.eqBand] ?? 'peak']
  const ref = useRef<HTMLCanvasElement>(null)
  const [corner, setCorner] = useState('')
  useEffect(() => {
    if (ref.current) setCorner(drawDisplay(ref.current, 'eq', v, ctx.fam, ctx.live, { eqBand: ctx.eqBand }))
  }, [v, ctx.fam, ctx.eqBand, ctx.live])

  // Drag a band's point: frequency across, gain up and down. Scroll for Q.
  const pick = (x: number, y: number, W: number, H: number) => {
    let best = -1, bestD = 18
    bands.forEach((bd, i) => {
      if (!bd.on && i !== ctx.eqBand) return
      const d = Math.hypot(freqX(bd.f, W) - x, (H / 2 - (bd.g / 24) * (H / 2 - 10)) - y)
      if (d < bestD) { bestD = d; best = i }
    })
    return best
  }
  const onPointerDown = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const c = e.currentTarget, r = c.getBoundingClientRect()
    const i = pick(e.clientX - r.left, e.clientY - r.top, r.width, r.height)
    if (i < 0) return
    ctx.setEqBand(i)
    const fq = v.par(`Band ${i + 1} Frequency`), gq = v.par(`Band ${i + 1} Gain`), en = v.par(`Band ${i + 1} Enabled`)
    if (!fq || !gq) return
    if (en && v.norm(en) < 0.5) ctx.setParam(en, en.max)
    c.setPointerCapture(e.pointerId)
    const move = (ev: PointerEvent) => {
      ctx.setParam(fq, xFreq(ev.clientX - r.left, r.width))
      ctx.setParam(gq, Math.max(-24, Math.min(24, yEq(ev.clientY - r.top, r.height))))
    }
    const up = () => { c.removeEventListener('pointermove', move); c.removeEventListener('pointerup', up) }
    c.addEventListener('pointermove', move); c.addEventListener('pointerup', up)
  }
  const onWheel = (e: React.WheelEvent<HTMLCanvasElement>) => {
    const qq = v.par(`Band ${b} Q`)
    if (!qq) return
    ctx.setParam(qq, v.v(`Band ${b} Q`, 0.7) * (e.deltaY < 0 ? 1.08 : 1 / 1.08))
  }
  const onContextMenu = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const r = e.currentTarget.getBoundingClientRect()
    const i = pick(e.clientX - r.left, e.clientY - r.top, r.width, r.height)
    const q = v.par(`Band ${(i < 0 ? ctx.eqBand : i) + 1} Gain`)
    if (q) ctx.openMenu(e, q)
  }

  return (
    <div className="cols" style={{ '--cols': '1fr 250px' } as CSSProperties}>
      <div className="col">
        <div className="pnl grow">
          <div className="ph"><span className="dot" /><span className="t">Response</span><span className="x">drag a point · scroll for Q</span></div>
          <div className="sec fill">
            <div className="disp drag" style={{ height: 300 }}>
              <canvas ref={ref} onPointerDown={onPointerDown} onWheel={onWheel} onContextMenu={onContextMenu} />
              <span className="dl">FREQUENCY RESPONSE</span>
              <span className="dr">{corner}</span>
            </div>
          </div>
          <div className="sec">
            <div className="bandbar">
              {bands.map((bd, i) => (
                <button key={i} style={{ '--c': EQ_BAND_COLOURS[i] } as CSSProperties} className={`${bd.on ? 'live' : ''} ${i === ctx.eqBand ? 'sel' : ''}`} onClick={() => ctx.setEqBand(i)}>
                  <i />BAND {i + 1}
                </button>
              ))}
            </div>
          </div>
        </div>
      </div>
      <div className="col">
        <div className="pnl" style={{ '--a': col, '--a2': col } as CSSProperties}>
          <div className="ph">
            <span className="dot" /><span className="t hi">Band {b}</span><span className="x">{kindName}</span>
            {onQ && (
              <span className={`tg${v.norm(onQ) >= 0.5 ? ' on' : ''}`} onClick={() => ctx.setParam(onQ, v.norm(onQ) >= 0.5 ? onQ.min : onQ.max)} onContextMenu={(e) => ctx.openMenu(e, onQ)}>ON<i /></span>
            )}
          </div>
          <div className="sec">
            <div className="rows">
              <div className="knobs"><KnobFor ctx={{ ...ctx, fam: { a: col, a2: col, deep: ctx.fam.deep } }} name={`Band ${b} Frequency`} big /></div>
              <div className="knobs">
                <KnobFor ctx={{ ...ctx, fam: { a: col, a2: col, deep: ctx.fam.deep } }} name={`Band ${b} Gain`} />
                <KnobFor ctx={{ ...ctx, fam: { a: col, a2: col, deep: ctx.fam.deep } }} name={`Band ${b} Q`} />
              </div>
            </div>
          </div>
        </div>
        <div className="pnl grow">
          <div className="ph"><span className="dot" /><span className="t">Output</span></div>
          <div className="sec fill"><div className="rows"><div className="knobs"><KnobFor ctx={ctx} name="Output Gain" big /></div><LevelRows live={ctx.live} /></div></div>
        </div>
      </div>
    </div>
  )
}

// --------------------------------------------------------------- presets

function PresetPicker({ trackId, slotId, pluginId, onLoaded }: { trackId: string; slotId: string; pluginId: string; onLoaded: () => void }) {
  const presets = usePluginPresetStore((s) => s.byPlugin[pluginId] ?? null)
  const currentId = usePluginPresetStore((s) => s.currentPresetId(trackId, slotId, pluginId))
  useEffect(() => { usePluginPresetStore.getState().refresh(pluginId).catch(() => { /* stays empty */ }) }, [pluginId])
  const report = (what: string) => (e: unknown) => useNotificationStore.getState().push('error', `${what}: ${String(e)}`)
  const current = presets?.find((p) => p.id === currentId)

  const step = async (d: 1 | -1) => {
    const p = await usePluginPresetStore.getState().step(trackId, slotId, pluginId, d).catch(report('Could not load the preset'))
    if (p) onLoaded()
  }
  const pick = async (value: string) => {
    if (!value) return
    if (value === '__save') {
      const name = window.prompt('Preset name:')?.trim()
      if (!name) return
      await usePluginPresetStore.getState().save(trackId, slotId, pluginId, name).catch(report('Could not save the preset'))
      return
    }
    await usePluginPresetStore.getState().load(trackId, slotId, pluginId, value).catch(report('Could not load the preset'))
    onLoaded()
  }
  const none = !presets?.length
  return (
    <>
      <button className="arw" onClick={() => void step(-1)} disabled={none} title="Previous preset">◀</button>
      <label className="preset" title="Presets">
        <select value="" onChange={(e) => void pick(e.target.value)}>
          <option value="">{current?.name ?? (presets == null ? 'Presets…' : none ? 'Init' : 'Presets')}</option>
          {(presets ?? []).map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
          <option value="__save">+ Save current as preset…</option>
        </select>
        <i>▾</i>
      </label>
      <button className="arw" onClick={() => void step(1)} disabled={none} title="Next preset">▶</button>
    </>
  )
}
