import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useNotificationStore } from '../../stores/notificationStore'
import { useTrackStore } from '../../stores/trackStore'
import { valueForNumber, type ParamView, type PluginParam } from './pluginDraw'

export interface ParamMenuTarget {
  x: number
  y: number
  param: PluginParam
}

/** Copied position (0..1) and its text, shared by every plug-in window here. */
let clipboard: { t: number; text: string } | null = null

const IC = {
  clip: '<rect x="1.5" y="3" width="13" height="10" rx="2"/><path d="M3.5 10.5 6 6.5l2.5 2.5 2-3.5 2 2"/>',
  lane: '<path d="M1.5 5h13M1.5 11h13"/><path d="M2 9.5 5 7l3 2 3-3 3 2"/>',
  link: '<path d="M6.5 9.5 9.5 6.5"/><path d="M7 4.5 8.5 3a2.5 2.5 0 0 1 3.5 3.5L10.5 8M9 11.5 7.5 13A2.5 2.5 0 0 1 4 9.5L5.5 8"/>',
  start: '<path d="M3 2.5v11M6 4l7 4-7 4z"/>',
  type: '<path d="M3 4h10M8 4v9"/>',
  reset: '<path d="M3 8a5 5 0 1 0 1.5-3.5"/><path d="M3 2.5V5h2.5"/>',
  copy: '<rect x="5" y="5" width="8.5" height="8.5" rx="1.5"/><path d="M3 10.5V3.5A1 1 0 0 1 4 2.5h7"/>',
  paste: '<rect x="3" y="3" width="10" height="11" rx="1.5"/><path d="M6 3V2h4v1"/>',
}

function Icon({ d }: { d: string }) {
  return <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.6} dangerouslySetInnerHTML={{ __html: d }} />
}

/**
 * Right-click on any control of a built-in plug-in, as in FL Studio: make an
 * automation clip for it, give it a lane, link it to a controller, start the
 * song with this value, or type, reset, copy and paste it.
 */
export function ParamMenu({ target, view, trackId, slotId, pluginName, accent, automated, onSet, onClose }: {
  target: ParamMenuTarget
  view: ParamView
  trackId: string
  slotId: string
  pluginName: string
  accent: string
  automated: boolean
  onSet: (value: number) => void
  onClose: () => void
}) {
  const q = target.param
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState({ left: target.x, top: target.y })
  const [typing, setTyping] = useState(false)
  const [draft, setDraft] = useState('')
  const inputRef = useRef<HTMLInputElement>(null)
  const notify = useNotificationStore.getState().push
  const t = view.norm(q)
  const current = view.text(q)
  const label = `${q.name} (${pluginName})`

  // Keep the menu on screen.
  useLayoutEffect(() => {
    const r = ref.current?.getBoundingClientRect()
    if (!r) return
    setPos({
      left: Math.max(8, Math.min(target.x, window.innerWidth - r.width - 8)),
      top: Math.max(8, Math.min(target.y, window.innerHeight - r.height - 8)),
    })
  }, [target.x, target.y, typing])

  useEffect(() => {
    const away = (e: PointerEvent) => { if (!ref.current?.contains(e.target as Node)) onClose() }
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('pointerdown', away, true)
    window.addEventListener('keydown', key)
    return () => { window.removeEventListener('pointerdown', away, true); window.removeEventListener('keydown', key) }
  }, [onClose])

  useEffect(() => { if (typing) inputRef.current?.select() }, [typing])

  const refresh = () => useTrackStore.getState().refreshTrack(trackId).catch(() => { /* the event catches up */ })

  const run = (fn: () => Promise<void> | void) => () => {
    onClose()
    Promise.resolve(fn()).catch((e) => notify('error', `${q.name}: ${String(e)}`))
  }

  const createClip = run(async () => {
    await invoke<string>('create_param_automation_clip', { trackId, slotId, paramId: q.id, value: t })
    await refresh()
    notify('info', `Automation clip for ${label}`, {
      detail: 'Eight bars from the playhead, on this channel, flat at the current value. Move its points to automate it.',
    })
  })

  const addLane = run(async () => {
    await invoke<string>('add_param_automation_lane', { trackId, slotId, paramId: q.id, value: t })
    await refresh()
    notify('info', `Automation lane for ${label}`, { detail: 'Under the channel in the playlist, starting at the current value.' })
  })

  const learn = run(async () => {
    await invoke('midi_learn_start', { target: { kind: 'pluginParam', trackId, slotId, paramId: q.id } })
    notify('info', 'Waiting for a knob', { detail: `Move a knob or fader on your controller to link it to ${label}.` })
  })

  const initSong = run(async () => {
    const changed = await invoke<number>('init_param_automation_at_start', { trackId, slotId, paramId: q.id, value: t })
    if (changed > 0) {
      await refresh()
      notify('info', `${label} starts the song at ${current}`)
    } else {
      notify('info', `${label} has no automation`, { detail: `It keeps ${current} from the start of the song already; it is saved with the song.` })
    }
  })

  const commitTyped = () => {
    const raw = draft.trim()
    onClose()
    if (!raw) return
    // A choice by its name.
    const opts = q.options ?? (view.isSwitch(q) ? ['Off', 'On'] : null)
    if (opts) {
      const i = opts.findIndex((o) => o.toLowerCase() === raw.toLowerCase())
      if (i >= 0) { onSet(view.denorm(q, opts.length > 1 ? i / (opts.length - 1) : 0)); return }
    }
    // A number in the parameter's unit: "2k", "-6", "120 ms", "1.5 s".
    const m = /^(-?[\d.]+)\s*(k)?\s*(hz|ms|s|db|%|st|ct|x)?$/i.exec(raw.replace(',', '.'))
    if (!m) { notify('warning', `"${raw}" is not a value for ${q.name}`); return }
    let n = parseFloat(m[1]) * (m[2] ? 1000 : 1)
    if (m[3]?.toLowerCase() === 's') n *= 1000
    onSet(valueForNumber(view, q, n))
  }

  return (
    <div
      ref={ref}
      className="ctx"
      style={{ left: pos.left, top: pos.top, '--a2': accent } as React.CSSProperties}
      onContextMenu={(e) => e.preventDefault()}
    >
      <div className="hd"><small>{pluginName} · {q.name}{automated ? ' · automated' : ''}</small><b>{current}</b></div>
      {typing ? (
        <div style={{ padding: '4px 6px 6px' }}>
          <input
            ref={inputRef}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') commitTyped(); else if (e.key === 'Escape') onClose() }}
            style={{
              width: '100%', font: '500 12px var(--mono)', color: '#fff', background: 'var(--faint)',
              border: '1px solid var(--a2)', borderRadius: 5, padding: '5px 8px', outline: 'none',
            }}
          />
          <div style={{ font: '500 10px var(--mono)', color: 'var(--dim)', marginTop: 4 }}>
            {q.texts ? `${q.texts[0]} to ${q.texts[100]}` : `${q.min} to ${q.max}`} · Enter to set
          </div>
        </div>
      ) : (
        <>
          <button className="pri" onClick={createClip}><Icon d={IC.clip} />Create automation clip</button>
          <button onClick={addLane}><Icon d={IC.lane} />Add automation lane on this channel</button>
          <button onClick={learn}><Icon d={IC.link} />Link to controller…<em>MIDI learn</em></button>
          <button onClick={initSong}><Icon d={IC.start} />Init song with this value</button>
          <hr />
          <button onClick={() => { setDraft(current); setTyping(true) }}><Icon d={IC.type} />Type in value…</button>
          <button onClick={run(() => onSet(q.defaultValue))}><Icon d={IC.reset} />Reset to default<em>{view.text(q, q.defaultValue)}</em></button>
          <button onClick={run(() => { clipboard = { t, text: current }; notify('info', `Copied ${current}`) })}><Icon d={IC.copy} />Copy value</button>
          <button
            onClick={run(() => { if (clipboard) onSet(view.denorm(q, clipboard.t)) })}
            disabled={!clipboard}
            style={clipboard ? undefined : { opacity: 0.4 }}
          >
            <Icon d={IC.paste} />Paste value{clipboard && <em>{clipboard.text}</em>}
          </button>
        </>
      )}
    </div>
  )
}
