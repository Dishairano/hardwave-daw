// VCA faders: one fader that moves a group of tracks together, each keeping
// its own balance, and a mute that silences the group without touching the
// tracks' own mute buttons.
//
// The engine applied them and the backend had every command; nothing in the
// interface called any of them, so there was no way to make one.

import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { useHistoryStore } from './historyStore'

export interface Vca {
  id: string
  name: string
  gain_db: number
  muted: boolean
  members: string[]
}

interface VcaState {
  vcas: Vca[]
  fetch: () => Promise<void>
  add: () => Promise<void>
  rename: (id: string, name: string) => Promise<void>
  remove: (id: string) => Promise<void>
  /** While the fader moves: heard at once, and not an undo step each. */
  setGainLive: (id: string, gainDb: number) => void
  setMuted: (id: string, muted: boolean) => Promise<void>
  setMembers: (id: string, members: string[]) => Promise<void>
}

export const useVcaStore = create<VcaState>((set, get) => ({
  vcas: [],

  fetch: async () => {
    try {
      set({ vcas: await invoke<Vca[]>('list_vcas') })
    } catch {
      // No engine in the browser preview: keep what is shown.
    }
  },

  add: async () => {
    const name = `VCA ${get().vcas.length + 1}`
    const since = Date.now()
    await invoke('add_vca', { name })
    useHistoryStore.getState().push(`Add ${name}`, since)
    await get().fetch()
  },

  rename: async (id, name) => {
    const since = Date.now()
    await invoke('rename_vca', { id, name })
    useHistoryStore.getState().push(`Rename VCA to "${name}"`, since)
    await get().fetch()
  },

  remove: async (id) => {
    const name = get().vcas.find(v => v.id === id)?.name ?? 'VCA'
    const since = Date.now()
    await invoke('delete_vca', { id })
    useHistoryStore.getState().push(`Delete ${name}`, since)
    await get().fetch()
  },

  setGainLive: (id, gainDb) => {
    set(s => ({ vcas: s.vcas.map(v => (v.id === id ? { ...v, gain_db: gainDb } : v)) }))
    invoke('set_vca_gain', { id, gainDb }).catch(() => {})
  },

  setMuted: async (id, muted) => {
    set(s => ({ vcas: s.vcas.map(v => (v.id === id ? { ...v, muted } : v)) }))
    await invoke('set_vca_muted', { id, muted })
  },

  setMembers: async (id, members) => {
    set(s => ({ vcas: s.vcas.map(v => (v.id === id ? { ...v, members } : v)) }))
    const since = Date.now()
    await invoke('set_vca_members', { id, members })
    useHistoryStore.getState().push('Change VCA members', since)
    await get().fetch()
  },
}))
