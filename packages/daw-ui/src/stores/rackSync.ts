import { invoke } from '@tauri-apps/api/core'
import { isHydrating, usePatternStore } from './patternStore'
import { useChannelRackPrefsStore } from './channelRackPrefsStore'
import { useHistoryStore } from './historyStore'
import { useProjectStore } from './projectStore'

/**
 * Keep the engine, and every other window, in step with the channel rack.
 *
 * The rack's patterns lived only in this window's store and were handed to
 * the project when the song was saved, so the step sequencer made no sound.
 * Now each change goes to the engine shortly after it is made (pattern mode
 * plays it), and the engine tells the other windows so a detached rack and
 * the main window never save each other's old steps.
 *
 * An edit is an undo step: step edits had none, so Ctrl+Z skipped them and
 * undid the edit before. A burst of edits (painting a row of steps) is one
 * step; a rack loaded from a song or another window is none.
 */
let started = false
let applyingRemote = false

function thisWindow(): string {
  const meta = (window as unknown as {
    __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } }
  }).__TAURI_INTERNALS__?.metadata
  return meta?.currentWindow?.label ?? 'main'
}

export function startRackSync(): void {
  if (started) return
  started = true
  let timer: ReturnType<typeof setTimeout> | null = null
  let edited = false
  const push = () => {
    if (applyingRemote) return
    if (!isHydrating()) edited = true
    if (timer) clearTimeout(timer)
    timer = setTimeout(() => {
      timer = null
      const undoable = edited
      edited = false
      invoke<boolean>('set_channel_rack_state', {
        payload: usePatternStore.getState().serialize(),
        from: thisWindow(),
        undoable,
      })
        .then((recorded) => {
          if (!recorded) return
          useProjectStore.getState().markDirty()
          useHistoryStore.getState().push('Edit the channel rack')
        })
        .catch(() => { /* outside the app */ })
    }, 120)
  }
  usePatternStore.subscribe((s, prev) => {
    if (s.patterns !== prev.patterns || s.activeId !== prev.activeId) push()
  })
  useChannelRackPrefsStore.subscribe((s, prev) => {
    if (s.globalSwing !== prev.globalSwing || s.channelSwingmix !== prev.channelSwingmix || s.patternLength !== prev.patternLength) push()
  })
  import('@tauri-apps/api/event')
    .then(({ listen }) =>
      listen<{ payload: string | null; from: string }>('daw:rackChanged', (ev) => {
        if (ev.payload.from === thisWindow()) return
        applyingRemote = true
        try {
          usePatternStore.getState().hydrate(ev.payload.payload)
        } finally {
          applyingRemote = false
        }
      }),
    )
    .catch(() => { started = false })
}
