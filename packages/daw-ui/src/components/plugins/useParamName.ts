import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useTrackStore, type AutomationTargetInfo } from '../../stores/trackStore'

/** Parameter names per slot, read once per session. */
const names = new Map<string, Promise<Map<number, string>>>()

function slotNames(trackId: string, slotId: string): Promise<Map<number, string>> {
  const key = `${trackId}\u0000${slotId}`
  let p = names.get(key)
  if (!p) {
    p = invoke<{ id: number; name: string }[]>('get_plugin_parameters', { trackId, slotId })
      .then((list) => new Map(list.map((q) => [q.id, q.name])))
      .catch(() => {
        names.delete(key)
        return new Map<number, string>()
      })
    names.set(key, p)
  }
  return p
}

/**
 * What an automation lane or clip drives, in words: "Cutoff · Filter" for a
 * plug-in's parameter. Lanes and clips said "Plugin p4", which named neither
 * the knob nor the plug-in.
 */
export function useAutomationTargetName(trackId: string, target: AutomationTargetInfo, plain: (t: AutomationTargetInfo) => string): string {
  const slotId = target.kind === 'plugin_param' ? target.slotId : null
  const paramId = target.kind === 'plugin_param' ? target.paramId : null
  const plugin = useTrackStore((s) => (slotId ? s.tracks.find((t) => t.id === trackId)?.inserts?.find((i) => i.id === slotId)?.pluginName : undefined))
  const [param, setParam] = useState<string | null>(null)
  useEffect(() => {
    if (!slotId || paramId == null) return
    let alive = true
    slotNames(trackId, slotId).then((m) => { if (alive) setParam(m.get(paramId) ?? null) })
    return () => { alive = false }
  }, [trackId, slotId, paramId])
  if (target.kind !== 'plugin_param') return plain(target)
  const who = plugin?.replace(/^Hardwave /, '')
  if (param && who) return `${param} · ${who}`
  return param ?? (who ? `${who} · parameter ${target.paramId}` : plain(target))
}
