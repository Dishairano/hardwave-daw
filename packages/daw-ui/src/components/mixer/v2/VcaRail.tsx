import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../../../theme'
import { useTrackStore } from '../../../stores/trackStore'
import { useNotificationStore } from '../../../stores/notificationStore'

/**
 * VCA groups, as a rail of slim faders beside the channels.
 *
 * Pulling a drum group down meant routing every drum into a bus, which
 * changes where the effects sit and what the sidechain keys off, or
 * dragging six faders and hoping the balance held. A VCA adds its level
 * to each member's own fader and moves nothing in the signal path.
 */

interface Vca {
  id: string
  name: string
  gain_db: number
  muted: boolean
  members: string[]
}

export function VcaRail() {
  const tracks = useTrackStore(s => s.tracks)
  const [vcas, setVcas] = useState<Vca[]>([])
  const [membersOf, setMembersOf] = useState<string | null>(null)

  const refresh = useCallback(async () => {
    try {
      setVcas(await invoke<Vca[]>('list_vcas'))
    } catch { /* no engine in the browser preview */ }
  }, [])

  useEffect(() => { void refresh() }, [refresh])

  const addGroup = useCallback(async () => {
    const name = window.prompt('Name this group', 'Drums')?.trim()
    if (!name) return
    try {
      await invoke('add_vca', { name })
      await refresh()
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not add that group', { detail: String(e) })
    }
  }, [refresh])

  const setGain = useCallback(async (id: string, gainDb: number) => {
    setVcas(prev => prev.map(v => (v.id === id ? { ...v, gain_db: gainDb } : v)))
    try {
      await invoke('set_vca_gain', { id, gainDb })
    } catch { /* the fader already moved; the next refresh corrects it */ }
  }, [])

  const toggleMute = useCallback(async (v: Vca) => {
    setVcas(prev => prev.map(x => (x.id === v.id ? { ...x, muted: !x.muted } : x)))
    try {
      await invoke('set_vca_muted', { id: v.id, muted: !v.muted })
    } catch { void refresh() }
  }, [refresh])

  const remove = useCallback(async (v: Vca) => {
    try {
      await invoke('delete_vca', { id: v.id })
      await refresh()
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not remove that group', { detail: String(e) })
    }
  }, [refresh])

  const rename = useCallback(async (v: Vca) => {
    const name = window.prompt('Rename this group', v.name)?.trim()
    if (!name || name === v.name) return
    try {
      await invoke('rename_vca', { id: v.id, name })
      await refresh()
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not rename that group', { detail: String(e) })
    }
  }, [refresh])

  const saveMembers = useCallback(async (id: string, members: string[]) => {
    try {
      await invoke('set_vca_members', { id, members })
      await refresh()
      setMembersOf(null)
    } catch (e) {
      useNotificationStore.getState().push('warning', 'Could not set those members', { detail: String(e) })
    }
  }, [refresh])

  const editing = vcas.find(v => v.id === membersOf)

  return (
    <div style={{
      display: 'flex', flexDirection: 'column', minWidth: 76, maxWidth: 232,
      borderRight: `1px solid ${hw.border}`, background: hw.bgElevated,
    }}>
      <div style={{
        display: 'flex', alignItems: 'center', gap: 4, padding: '4px 6px',
        borderBottom: `1px solid ${hw.border}`,
      }}>
        <span style={{ fontSize: 8, color: hw.textFaint, letterSpacing: 0.5 }}>VCA</span>
        <div style={{ flex: 1 }} />
        <button onClick={() => { void addGroup() }} title="New VCA group" style={tinyBtn()}>+</button>
      </div>

      <div style={{ display: 'flex', overflowX: 'auto', flex: 1 }}>
        {vcas.length === 0 && (
          <div style={{ padding: 8, fontSize: 9, color: hw.textFaint, width: 70 }}>
            No groups. Add one, then pick the tracks it rides.
          </div>
        )}
        {vcas.map(v => (
          <div
            key={v.id}
            style={{
              width: 70, padding: '6px 4px', display: 'flex', flexDirection: 'column',
              alignItems: 'center', gap: 4, borderRight: `1px solid ${hw.border}`,
            }}
          >
            <div
              onDoubleClick={() => { void rename(v) }}
              title={`${v.members.length} track${v.members.length === 1 ? '' : 's'} — double-click to rename`}
              style={{
                fontSize: 9, color: hw.textPrimary, maxWidth: 62,
                overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                cursor: 'pointer',
              }}
            >{v.name}</div>

            <input
              className="hw-fader"
              type="range"
              min={-60} max={12} step={0.1}
              value={v.gain_db}
              onChange={(e) => { void setGain(v.id, Number(e.target.value)) }}
              style={{
                writingMode: 'vertical-lr', direction: 'rtl',
                height: 120, accentColor: hw.accent,
              }}
            />
            <div style={{ fontSize: 8, color: hw.textSecondary, fontVariantNumeric: 'tabular-nums' }}>
              {v.gain_db > 0 ? '+' : ''}{v.gain_db.toFixed(1)}
            </div>

            <button
              onClick={() => { void toggleMute(v) }}
              title="Silence every track in this group"
              style={{ ...tinyBtn(v.muted), width: 28 }}
            >M</button>
            <button
              onClick={() => setMembersOf(v.id)}
              title="Choose the tracks this group rides"
              style={tinyBtn()}
            >{v.members.length}</button>
            <button onClick={() => { void remove(v) }} title="Remove this group" style={tinyBtn()}>x</button>
          </div>
        ))}
      </div>

      {editing && (
        <MemberPicker
          key={editing.id}
          name={editing.name}
          all={tracks.map(t => ({ id: t.id, name: t.name, kind: t.kind }))}
          initial={editing.members}
          onCancel={() => setMembersOf(null)}
          onSave={(members) => { void saveMembers(editing.id, members) }}
        />
      )}
    </div>
  )
}

function MemberPicker({
  name, all, initial, onCancel, onSave,
}: {
  name: string
  all: { id: string; name: string; kind: string }[]
  initial: string[]
  onCancel: () => void
  onSave: (members: string[]) => void
}) {
  const [chosen, setChosen] = useState<string[]>(initial)
  // The master is left out: a group riding the master is the master fader
  // with extra steps, and muting it would look like the DAW had died.
  const candidates = all.filter(t => t.kind !== 'Master')

  return (
    <div
      style={{
        position: 'fixed', inset: 0, zIndex: 10001,
        background: 'rgba(0,0,0,0.45)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
      onMouseDown={(e) => { if (e.target === e.currentTarget) onCancel() }}
    >
      <div style={{
        width: 360, maxWidth: '92vw', maxHeight: '70vh',
        background: hw.bg, color: hw.textPrimary,
        border: `1px solid ${hw.border}`, borderRadius: hw.radius.lg,
        overflow: 'hidden', display: 'flex', flexDirection: 'column',
      }}>
        <div style={{ padding: '8px 12px', background: hw.bgElevated, borderBottom: `1px solid ${hw.border}` }}>
          <div style={{ fontSize: 12, fontWeight: 600 }}>Tracks "{name}" rides</div>
          <div style={{ fontSize: 9, color: hw.textFaint, marginTop: 2 }}>
            Their own faders stay where they are. The group's level is added on top.
          </div>
        </div>
        <div style={{ overflowY: 'auto', flex: 1, padding: 6 }}>
          {candidates.map(t => (
            <label key={t.id} style={{
              display: 'flex', alignItems: 'center', gap: 8,
              padding: '4px 8px', fontSize: 11, cursor: 'pointer',
            }}>
              <input
                type="checkbox"
                checked={chosen.includes(t.id)}
                onChange={() => setChosen(prev =>
                  prev.includes(t.id) ? prev.filter(x => x !== t.id) : [...prev, t.id])}
                style={{ accentColor: hw.accent }}
              />
              <span>{t.name}</span>
            </label>
          ))}
        </div>
        <div style={{
          padding: '8px 12px', display: 'flex', gap: 8, justifyContent: 'flex-end',
          background: hw.bgElevated, borderTop: `1px solid ${hw.border}`,
        }}>
          <button onClick={onCancel} style={tinyBtn()}>Cancel</button>
          <button onClick={() => onSave(chosen)} style={tinyBtn(true)}>Save</button>
        </div>
      </div>
    </div>
  )
}

function tinyBtn(active: boolean = false) {
  return {
    padding: '2px 8px', fontSize: 9, background: 'transparent',
    border: `1px solid ${active ? hw.accent : hw.border}`, borderRadius: hw.radius.sm,
    color: active ? hw.accent : hw.textSecondary, cursor: 'pointer',
  } as const
}
