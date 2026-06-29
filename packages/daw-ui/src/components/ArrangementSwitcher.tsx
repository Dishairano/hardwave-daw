/*
 * ArrangementSwitcher — pick / create / rename / delete the project's
 * arrangements (FL Studio-style switchable playlists). Switching swaps
 * the playlist timeline; we refetch tracks so the playlist re-renders.
 */
import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useTrackStore } from '../stores/trackStore'

interface ArrangementInfo {
  id: string
  name: string
  active: boolean
}

export function ArrangementSwitcher() {
  const [arrangements, setArrangements] = useState<ArrangementInfo[]>([])
  const fetchTracks = useTrackStore(s => s.fetchTracks)

  const refresh = useCallback(async () => {
    try {
      setArrangements(await invoke<ArrangementInfo[]>('list_arrangements'))
    } catch (e) {
      console.warn('list_arrangements failed', e)
    }
  }, [])

  useEffect(() => {
    refresh()
  }, [refresh])

  const active = arrangements.find(a => a.active)

  const onSwitch = useCallback(
    async (id: string) => {
      if (id === active?.id) return
      try {
        await invoke('switch_arrangement', { id })
        await fetchTracks()
        await refresh()
      } catch (e) {
        console.warn('switch_arrangement failed', e)
      }
    },
    [active?.id, fetchTracks, refresh],
  )

  const onCreate = useCallback(async () => {
    const name = `Arrangement ${arrangements.length + 1}`
    try {
      await invoke('create_arrangement', { name, copyCurrent: false })
      await fetchTracks()
      await refresh()
    } catch (e) {
      console.warn('create_arrangement failed', e)
    }
  }, [arrangements.length, fetchTracks, refresh])

  const onDuplicate = useCallback(async () => {
    const name = `${active?.name ?? 'Arrangement'} (copy)`
    try {
      await invoke('create_arrangement', { name, copyCurrent: true })
      await fetchTracks()
      await refresh()
    } catch (e) {
      console.warn('duplicate arrangement failed', e)
    }
  }, [active?.name, fetchTracks, refresh])

  const onRename = useCallback(async () => {
    if (!active) return
    const name = window.prompt('Rename arrangement', active.name)?.trim()
    if (!name) return
    try {
      await invoke('rename_arrangement', { id: active.id, name })
      await refresh()
    } catch (e) {
      console.warn('rename_arrangement failed', e)
    }
  }, [active, refresh])

  const onDelete = useCallback(async () => {
    if (!active || arrangements.length <= 1) return
    if (!window.confirm(`Delete arrangement "${active.name}"?`)) return
    try {
      await invoke('delete_arrangement', { id: active.id })
      await fetchTracks()
      await refresh()
    } catch (e) {
      console.warn('delete_arrangement failed', e)
    }
  }, [active, arrangements.length, fetchTracks, refresh])

  const btn: React.CSSProperties = {
    fontSize: 10, fontWeight: 700, padding: '2px 6px', cursor: 'pointer',
    background: 'rgba(255,255,255,0.06)', color: '#cdcdd6',
    border: '1px solid #2a2a36', borderRadius: 4,
  }

  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 4, padding: '3px 6px' }}>
      <span style={{ fontSize: 8, letterSpacing: 0.5, textTransform: 'uppercase', color: '#7a7a88' }}>Arr</span>
      <select
        value={active?.id ?? ''}
        onChange={e => onSwitch(e.target.value)}
        title="Switch arrangement"
        style={{
          fontSize: 10, background: 'rgba(255,255,255,0.05)', color: '#e8e8ef',
          border: '1px solid #2a2a36', borderRadius: 4, padding: '2px 4px', maxWidth: 160,
        }}
      >
        {arrangements.map(a => (
          <option key={a.id} value={a.id}>{a.name}</option>
        ))}
      </select>
      <button type="button" style={btn} title="New arrangement" onClick={onCreate}>+</button>
      <button type="button" style={btn} title="Duplicate arrangement" onClick={onDuplicate}>dup</button>
      <button type="button" style={btn} title="Rename arrangement" onClick={onRename}>ren</button>
      <button
        type="button"
        style={{ ...btn, opacity: arrangements.length <= 1 ? 0.4 : 1 }}
        title="Delete arrangement"
        onClick={onDelete}
        disabled={arrangements.length <= 1}
      >×</button>
    </div>
  )
}
