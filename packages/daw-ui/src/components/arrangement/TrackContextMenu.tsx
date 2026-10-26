// The track menu, opened by right-clicking a track's name in the playlist.
//
// It lived in the old track list. When the playlist got its new track header
// that list stopped being shown anywhere, and everything only this menu could
// reach went with it: MIDI effects, sending a track's notes to another, MPE,
// Scala tunings, spreading takes onto lanes, lining two tracks up, freezing,
// bouncing, solo-safe and saving a track template. All of it still worked;
// there was just no way in.

import { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import { invoke } from '@tauri-apps/api/core'
import { MidiRouteDialog } from './MidiRouteDialog'
import { MidiFxPanel } from '../MidiFxPanel'
import { hw } from '../../theme'
import { useTrackStore } from '../../stores/trackStore'
import { PATTERN_COLORS } from '../../stores/patternStore'
import { useTrackFolderStore } from '../../stores/trackFolderStore'
import { useNotificationStore } from '../../stores/notificationStore'
import { useColorPickerStore } from '../../stores/colorPickerStore'

export function TrackContextMenu({ trackId, x, y, onClose }: {
  trackId: string
  x: number
  y: number
  onClose: () => void
}) {
  const tracks = useTrackStore(s => s.tracks)
  const t = tracks.find(tr => tr.id === trackId)
  const folders = useTrackFolderStore(s => s.folders)
  const [dialog, setDialog] = useState<'midiFx' | 'route' | null>(null)
  const [frozen, setFrozen] = useState(false)
  const [tuning, setTuning] = useState<string | null>(null)
  const [mpe, setMpe] = useState(false)
  const isMidi = t?.kind === 'Midi'

  // What the menu has to say about this one track, read as it opens.
  useEffect(() => {
    invoke<string[]>('list_frozen_tracks').then(ids => setFrozen(ids.includes(trackId))).catch(() => {})
    if (!isMidi) return
    invoke<{ name: string } | null>('get_tuning', { trackId }).then(v => setTuning(v?.name ?? null)).catch(() => {})
    invoke<boolean>('get_mpe', { trackId }).then(setMpe).catch(() => {})
  }, [trackId, isMidi])

  useEffect(() => {
    if (dialog) return
    const close = (e: MouseEvent) => {
      if ((e.target as HTMLElement).closest('[data-track-ctx-menu]')) return
      onClose()
    }
    const escape = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('mousedown', close)
    window.addEventListener('keydown', escape)
    return () => {
      window.removeEventListener('mousedown', close)
      window.removeEventListener('keydown', escape)
    }
  }, [dialog, onClose])

  if (!t) return null
  // On the page itself: the track list it opens from is moved with a
  // transform, which would pin a fixed menu to the list instead.
  if (dialog === 'midiFx') return createPortal(<MidiFxPanel trackId={t.id} onClose={onClose} />, document.body)
  if (dialog === 'route') return createPortal(<MidiRouteDialog trackId={t.id} onClose={onClose} />, document.body)

  const store = useTrackStore.getState()
  const push = useNotificationStore.getState().push
  const run = (action: () => Promise<void> | void) => () => {
    onClose()
    Promise.resolve(action()).catch(e => push('warning', 'That did not work', { detail: String(e) }))
  }
  const folderOf = folders.find(f => f.trackIds.includes(t.id))
  const otherAudio = tracks.filter(x => x.id !== t.id && x.kind === 'Audio')

  return createPortal(
    <div
      data-track-ctx-menu
      onMouseDown={e => e.stopPropagation()}
      onContextMenu={e => e.preventDefault()}
      style={{
        position: 'fixed', left: Math.min(x, window.innerWidth - 240), top: Math.min(y, window.innerHeight - 560),
        zIndex: 10000, minWidth: 220, padding: 4,
        background: 'rgba(12,12,18,0.96)',
        border: `1px solid ${hw.borderLight}`,
        borderRadius: hw.radius.md,
        boxShadow: '0 8px 32px rgba(0,0,0,0.55)',
        backdropFilter: hw.blur.md,
      }}
    >
      <Heading text={t.name} />
      <Item label="Rename" shortcut="F2" onClick={run(async () => {
        const name = window.prompt('Track name:', t.name)?.trim()
        if (name && name !== t.name) await store.renameTrack(t.id, name)
      })} />
      <Item label={t.muted ? 'Unmute' : 'Mute'} onClick={run(() => store.toggleMute(t.id))} />
      <Item label={t.soloed ? 'Unsolo' : 'Solo'} onClick={run(() => store.toggleSolo(t.id))} />
      <Item label={t.solo_safe ? 'Solo-safe: on' : 'Solo-safe: off'} onClick={run(() => store.toggleSoloSafe(t.id))} />

      {isMidi && (
        <>
          <Divider />
          <Item label="MIDI effects…" onClick={() => setDialog('midiFx')} />
          <Item label="Send its notes to…" onClick={() => setDialog('route')} />
          <Item label={mpe ? 'MPE: on' : 'MPE: off'} onClick={run(async () => {
            await invoke('set_mpe', { trackId: t.id, enabled: !mpe })
            push('info', !mpe ? `"${t.name}" reads MPE` : `"${t.name}" reads ordinary MIDI`, {
              detail: !mpe
                ? 'Each note is bent and pressed on the channel it arrives on, so one note of a chord can be bent on its own.'
                : 'One bend for the whole track again.',
            })
          })} />
          <Item label={tuning ? `Tuning: ${tuning} (clear)` : 'Tuning from a Scala file…'} onClick={run(async () => {
            if (tuning) {
              await invoke('clear_tuning', { trackId: t.id })
              push('info', `"${t.name}" is back in twelve equal`)
              return
            }
            const { open } = await import('@tauri-apps/plugin-dialog')
            const picked = await open({ multiple: false, filters: [{ name: 'Scala tuning', extensions: ['scl'] }] })
            if (typeof picked !== 'string') return
            const loaded = await invoke<{ name: string; degrees_cents: number[] }>(
              'load_tuning_file', { trackId: t.id, path: picked, rootNote: null, rootHz: null },
            )
            push('info', `"${t.name}" plays ${loaded.name}`, {
              detail: `${loaded.degrees_cents.length} notes to the repeat. The built-in instruments follow it; a hosted plug-in keeps its own tuning.`,
            })
          })} />
        </>
      )}

      <Divider />
      <Item label="Spread the takes onto lanes" onClick={run(async () => {
        const lanes = await invoke<number>('spread_takes_to_lanes', { trackId: t.id })
        await store.fetchTracks()
        push('info', lanes > 1 ? `${lanes} takes on their own lanes` : 'Only one take on this track')
      })} />
      {t.kind === 'Audio' && (
        <Item label="Line up with another track…" onClick={run(async () => {
          if (otherAudio.length === 0) { push('info', 'There is no other audio track to line up with'); return }
          const names = otherAudio.map(o => o.name)
          const picked = window.prompt(`Line "${t.name}" up with which track?\n\n${names.join('\n')}`, names[0])?.trim()
          const reference = otherAudio.find(o => o.name === picked)
          if (!reference) return
          const result = await invoke<{ offsetSamples: number; offsetMs: number; polarityFlipped: boolean }>(
            'align_track_to', { trackId: t.id, referenceTrackId: reference.id },
          )
          await store.fetchTracks()
          const gap = `${Math.abs(result.offsetMs).toFixed(2)} ms`
          push('info', `"${t.name}" lined up with "${reference.name}"`, {
            detail: (result.offsetSamples === 0
              ? 'They were already in time'
              : result.offsetMs > 0
                ? `"${t.name}" was ${gap} late, so "${reference.name}" now waits for it`
                : `"${t.name}" was ${gap} early and now waits for "${reference.name}"`)
              + `${result.polarityFlipped ? ', and it was turned upside down because the two agreed better that way' : ''}.`,
          })
        })} />
      )}

      <Divider />
      <Item label={frozen ? 'Unfreeze' : 'Freeze'} onClick={run(async () => {
        if (frozen) {
          await invoke('unfreeze_track', { trackId: t.id })
          push('info', `"${t.name}" is live again`, { detail: 'The part, the plug-ins and the automation are exactly as they were.' })
        } else {
          push('info', `Freezing "${t.name}"…`)
          await invoke<string>('freeze_track', { trackId: t.id })
          push('info', `"${t.name}" is frozen`, { detail: 'It plays a render now, so its plug-ins cost nothing until you unfreeze it.' })
        }
        await store.fetchTracks()
      })} />
      <Item label="Bounce to audio" onClick={run(async () => {
        push('info', `Bouncing "${t.name}"…`)
        await invoke<string>('bounce_track_to_audio', { trackId: t.id })
        await store.fetchTracks()
        push('info', `Bounced "${t.name}"`, { detail: 'The original is muted. Unmute it to go back to the plug-ins.' })
      })} />

      <Divider />
      <Heading text="Color" />
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(6, 1fr)', gap: 2, padding: '2px 6px 4px' }}>
        {PATTERN_COLORS.map(c => (
          <button key={c} title={c} onClick={run(() => store.setTrackColor(t.id, c))}
            style={{
              width: 18, height: 18, borderRadius: hw.radius.sm, padding: 0, cursor: 'pointer',
              background: c, border: c === t.color ? '2px solid #fff' : '1px solid rgba(255,255,255,0.12)',
            }}
          />
        ))}
      </div>
      <Item label="Custom color…" onClick={(e) => {
        const anchor = (e.currentTarget as HTMLElement).getBoundingClientRect()
        onClose()
        useColorPickerStore.getState().open({
          anchor, current: t.color, title: `Color · ${t.name}`,
          onPick: (c) => { store.setTrackColor(t.id, c).catch(console.error) },
        })
      }} />

      <Divider />
      <Item label="New folder from this track" onClick={run(() => {
        const name = window.prompt('Folder name:', `Group ${folders.length + 1}`)?.trim()
        if (name) useTrackFolderStore.getState().createFolder([t.id], name)
      })} />
      {folders.filter(f => f.id !== folderOf?.id).map(f => (
        <Item key={f.id} label={`Add to "${f.name}"`} onClick={run(() => useTrackFolderStore.getState().addTrackToFolder(t.id, f.id))} />
      ))}
      {folderOf && (
        <Item label="Remove from folder" onClick={run(() => useTrackFolderStore.getState().removeTrackFromFolder(t.id))} />
      )}
      <Item label="Save as track template…" onClick={run(() => {
        const name = window.prompt('Template name:', t.name)?.trim()
        if (!name) return
        // The track with its plug-ins and their settings, and the bus and
        // sends it feeds; Add > Track from a template brings it back.
        invoke('save_track_template', { trackId: t.id, name })
          .then(() => push('info', `Saved the track template "${name}"`, { detail: 'Add > Track from a template adds it to a song.' }))
          .catch(err => push('warning', 'The track template was not saved', { detail: String(err) }))
      })} />

      <Divider />
      <Item label="Delete" shortcut="Del" danger onClick={run(() => store.removeTrack(t.id))} />
    </div>,
    document.body,
  )
}

function Heading({ text }: { text: string }) {
  return (
    <div style={{ padding: '4px 8px 2px', fontSize: 8, color: hw.textFaint, letterSpacing: 0.5, textTransform: 'uppercase' }}>
      {text}
    </div>
  )
}

function Divider() {
  return <div style={{ height: 1, background: hw.border, margin: '3px 0' }} />
}

function Item({ label, shortcut, danger, onClick }: {
  label: string
  shortcut?: string
  danger?: boolean
  onClick: (e: React.MouseEvent) => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        width: '100%', display: 'flex', alignItems: 'center',
        padding: '5px 8px', gap: 8, border: 'none',
        background: 'transparent', color: danger ? hw.red : hw.textSecondary,
        fontSize: 11, cursor: 'pointer', borderRadius: hw.radius.sm,
      }}
      onMouseEnter={e => { (e.currentTarget as HTMLElement).style.background = 'rgba(255,255,255,0.05)' }}
      onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = 'transparent' }}
    >
      <span style={{ flex: 1, textAlign: 'left' }}>{label}</span>
      {shortcut && <span style={{ fontSize: 9, color: hw.textFaint }}>{shortcut}</span>}
    </button>
  )
}
