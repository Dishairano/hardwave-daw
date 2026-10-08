import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../../theme'

interface PluginParamInfo {
  id: number
  name: string
  defaultValue: number
  value: number
  min: number
  max: number
  unit: string
  automatable: boolean
}

function formatParamValue(v: number, unit: string): string {
  const n = Number.isInteger(v) ? String(v) : v.toFixed(2)
  return unit ? `${n} ${unit}` : n
}

/**
 * Generic parameter sheet — the fallback editor for plug-ins without a
 * custom GUI (and a handy automatable-param list even for those that do).
 * Fetches the slot's parameters via `get_plugin_parameters` and renders
 * each as a labelled slider that pushes edits through `set_plugin_parameter`.
 */
export function PluginParamSheet({ trackId, slotId }: { trackId: string; slotId: string }) {
  const [params, setParams] = useState<PluginParamInfo[]>([])
  const [values, setValues] = useState<Record<number, number>>({})
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    let cancelled = false
    setLoading(true)
    invoke<PluginParamInfo[]>('get_plugin_parameters', { trackId, slotId })
      .then((p) => {
        if (cancelled) return
        setParams(p)
        const init: Record<number, number> = {}
        p.forEach((x) => { init[x.id] = x.value })
        setValues(init)
      })
      .catch((e) => console.error('get_plugin_parameters failed', e))
      .finally(() => { if (!cancelled) setLoading(false) })
    return () => { cancelled = true }
  }, [trackId, slotId])

  const setParam = (id: number, v: number) => {
    setValues((prev) => ({ ...prev, [id]: v }))
    invoke('set_plugin_parameter', { trackId, slotId, paramId: id, value: v })
      .catch((e) => console.error('set_plugin_parameter failed', e))
  }

  if (loading) return <div style={{ fontSize: 10, color: hw.textFaint }}>Loading parameters…</div>
  if (params.length === 0) {
    return <div style={{ fontSize: 10, color: hw.textFaint }}>This plug-in exposes no adjustable parameters.</div>
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 7, maxHeight: 280, overflowY: 'auto', paddingRight: 2 }}>
      {params.map((p) => {
        const val = values[p.id] ?? p.value
        const span = p.max - p.min || 1
        return (
          <label
            key={p.id}
            data-hint={`${p.name}: ${formatParamValue(val, p.unit)}  (${p.min}–${p.max}${p.unit ? ' ' + p.unit : ''}) · double-click to reset`}
            style={{ display: 'flex', flexDirection: 'column', gap: 2 }}
          >
            <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 10, color: hw.textMuted, gap: 8 }}>
              <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{p.name}</span>
              <span style={{ fontFamily: 'var(--mono)', color: hw.textPrimary, flexShrink: 0 }}>{formatParamValue(val, p.unit)}</span>
            </div>
            <input
              type="range"
              min={p.min}
              max={p.max}
              step={span / 1000}
              value={val}
              onChange={(e) => setParam(p.id, Number(e.target.value))}
              onDoubleClick={() => setParam(p.id, p.defaultValue)}
              title={`${p.name} · double-click resets to default`}
              style={{ width: '100%', cursor: 'pointer' }}
            />
          </label>
        )
      })}
    </div>
  )
}
