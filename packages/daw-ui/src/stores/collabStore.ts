import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

/**
 * The room, as the arrangement needs it: whether there is one, and
 * where the other person is working.
 *
 * Polled only while a room is open, and lightly, because a cursor that
 * is half a second late is fine and a poll that runs all the time when
 * nobody is collaborating is not.
 */

export interface PeerCursor {
  name: string
  tick: number
  trackIndex: number | null
  panel: string
}

interface CollabState {
  connected: boolean
  peer: PeerCursor | null
  members: string[]
  /** Someone waiting for this side, the host, to let them in. */
  joinRequest: { requestId: string; name: string } | null
  start: () => void
  stop: () => void
  /** Tell the other person where you are, at most four times a second. */
  share: (tick: number, trackIndex: number | null, panel: string) => void
}

let timer: ReturnType<typeof setInterval> | null = null
let lastShared = 0

export const useCollabStore = create<CollabState>((set, get) => ({
  connected: false,
  peer: null,
  members: [],
  joinRequest: null,
  start: () => {
    if (timer) return
    const poll = () => {
      invoke<{
        connected: boolean
        peer: PeerCursor | null
        members: string[]
        joinRequest: { requestId: string; name: string } | null
      }>('collab_status')
        .then(status => {
          set({
            connected: status.connected,
            peer: status.peer,
            members: status.members,
            joinRequest: status.joinRequest ?? null,
          })
          if (!status.connected) get().stop()
        })
        .catch(() => { /* no engine in the browser preview */ })
    }
    poll()
    timer = setInterval(poll, 500)
  },
  stop: () => {
    if (timer) clearInterval(timer)
    timer = null
    set({ connected: false, peer: null, members: [], joinRequest: null })
  },
  share: (tick, trackIndex, panel) => {
    if (!get().connected) return
    const now = Date.now()
    if (now - lastShared < 250) return
    lastShared = now
    invoke('share_presence', { tick: Math.max(0, Math.round(tick)), trackIndex, panel }).catch(() => {})
  },
}))
