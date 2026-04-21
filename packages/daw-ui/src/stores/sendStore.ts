import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { useProjectStore } from './projectStore'
import { useHistoryStore } from './historyStore'

async function mut<T>(cmd: string, args?: Record<string, unknown>, label?: string): Promise<T> {
  const r = await invoke<T>(cmd, args)
  useProjectStore.getState().markDirty()
  if (label) useHistoryStore.getState().push(label)
  return r
}

export interface SendInfo {
  index: number
  target: string
  gainDb: number
  preFader: boolean
  enabled: boolean
}

export interface SendEdge {
  source: string
  target: string
  index: number
  gainDb: number
  preFader: boolean
  enabled: boolean
}

interface SendState {
  byTrack: Record<string, SendInfo[]>
  all: SendEdge[]
  fetchAll: () => Promise<void>
  fetchForTrack: (trackId: string) => Promise<SendInfo[]>
  addSend: (trackId: string, targetId: string, gainDb?: number, preFader?: boolean) => Promise<number>
  removeSend: (trackId: string, sendIndex: number) => Promise<void>
  setTarget: (trackId: string, sendIndex: number, targetId: string) => Promise<void>
  setGain: (trackId: string, sendIndex: number, gainDb: number) => Promise<void>
  setPreFader: (trackId: string, sendIndex: number, preFader: boolean) => Promise<void>
  setEnabled: (trackId: string, sendIndex: number, enabled: boolean) => Promise<void>
  createReturnWithSend: (sourceTrackId: string, returnName: string) => Promise<string>
}

export const useSendStore = create<SendState>((set, get) => ({
  byTrack: {},
  all: [],

  fetchAll: async () => {
    const all = await invoke<SendEdge[]>('list_sends')
    const byTrack: Record<string, SendInfo[]> = {}
    for (const e of all) {
      if (!byTrack[e.source]) byTrack[e.source] = []
      byTrack[e.source].push({
        index: e.index,
        target: e.target,
        gainDb: e.gainDb,
        preFader: e.preFader,
        enabled: e.enabled,
      })
    }
    set({ all, byTrack })
  },

  fetchForTrack: async (trackId) => {
    const sends = await invoke<SendInfo[]>('get_sends', { trackId })
    set(s => ({ byTrack: { ...s.byTrack, [trackId]: sends } }))
    return sends
  },

  addSend: async (trackId, targetId, gainDb, preFader) => {
    const idx = await mut<number>('add_send', {
      trackId, targetId,
      gainDb: gainDb ?? 0,
      preFader: preFader ?? false,
    }, 'Add send')
    await get().fetchAll()
    return idx
  },

  removeSend: async (trackId, sendIndex) => {
    await mut('remove_send', { trackId, sendIndex }, 'Remove send')
    await get().fetchAll()
  },

  setTarget: async (trackId, sendIndex, targetId) => {
    await mut('set_send_target', { trackId, sendIndex, targetId }, 'Change send target')
    await get().fetchAll()
  },

  setGain: async (trackId, sendIndex, gainDb) => {
    await mut('set_send_gain', { trackId, sendIndex, gainDb }, `Set send gain to ${gainDb.toFixed(1)} dB`)
    await get().fetchAll()
  },

  setPreFader: async (trackId, sendIndex, preFader) => {
    await mut('set_send_pre_fader', { trackId, sendIndex, preFader }, preFader ? 'Switch send to pre-fader' : 'Switch send to post-fader')
    await get().fetchAll()
  },

  setEnabled: async (trackId, sendIndex, enabled) => {
    await mut('set_send_enabled', { trackId, sendIndex, enabled }, enabled ? 'Enable send' : 'Disable send')
    await get().fetchAll()
  },

  createReturnWithSend: async (sourceTrackId, returnName) => {
    const newId = await mut<string>('create_return_with_send', { sourceTrackId, returnName }, `Add return "${returnName}"`)
    await get().fetchAll()
    return newId
  },
}))
