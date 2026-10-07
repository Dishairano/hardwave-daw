import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'
import { useTrackStore } from '../stores/trackStore'
import { useNotificationStore } from '../stores/notificationStore'
import { useCollabStore } from '../stores/collabStore'

/**
 * Working on a song with someone else.
 *
 * One of you opens a room and reads the code out; the other types it
 * in. What crosses the network is the edits, not the audio, so both
 * of you hear your own machine at full quality and a bad connection
 * costs a late edit rather than a dropout.
 *
 * Opening a room is part of Pro. Joining one is free, which is how
 * one subscription brings a second producer in.
 */

interface CollabStatus {
  connected: boolean
  roomId: string
  inviteCode: string
  hosting: boolean
  message: string
  received: number
  sent: number
  members: string[]
}

interface Joined {
  roomId: string
  inviteCode: string
  caughtUp: boolean
}

export function CollabPanel({ onClose }: { onClose: () => void }) {
  const [status, setStatus] = useState<CollabStatus | null>(null)
  const [code, setCode] = useState('')
  const [room, setRoom] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(async () => {
    try {
      setStatus(await invoke<CollabStatus>('collab_status'))
    } catch { /* no engine in the browser preview */ }
  }, [])

  useEffect(() => {
    void refresh()
    const id = setInterval(() => { void refresh() }, 1000)
    return () => clearInterval(id)
  }, [refresh])

  const open = useCallback(async () => {
    setBusy(true); setError(null)
    try {
      const joined = await invoke<Joined>('start_collab', {})
      setStatus(s => s && { ...s, connected: true, inviteCode: joined.inviteCode, hosting: true })
      useCollabStore.getState().start()
      useNotificationStore.getState().push('info', 'Room open', {
        detail: `Read this out: ${joined.inviteCode}`,
      })
      void refresh()
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }, [refresh])

  const join = useCallback(async () => {
    setBusy(true); setError(null)
    try {
      const joined = await invoke<Joined>('start_collab', { room: room.trim(), code: code.trim() })
      useCollabStore.getState().start()
      await useTrackStore.getState().fetchTracks()
      useNotificationStore.getState().push('info', 'You are in the room', {
        detail: joined.caughtUp
          ? 'Everything you missed has been applied.'
          : 'You were away too long to replay: ask them to send the project.',
      })
      void refresh()
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }, [room, code, refresh])

  const leave = useCallback(async () => {
    try { await invoke('stop_collab') } catch { /* already gone */ }
    useCollabStore.getState().stop()
    void refresh()
  }, [refresh])

  const connected = status?.connected ?? false

  return (
    <div
      style={{
        position: 'fixed', inset: 0, zIndex: 9800,
        background: 'rgba(0,0,0,0.45)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
      onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}
    >
      <div style={{
        width: 480, maxWidth: '94vw',
        background: hw.bg, color: hw.textPrimary,
        border: `1px solid ${hw.border}`, borderRadius: hw.radius.lg, overflow: 'hidden',
      }}>
        <div style={{
          padding: '8px 12px', display: 'flex', alignItems: 'center', gap: 12,
          background: hw.bgElevated, borderBottom: `1px solid ${hw.border}`,
        }}>
          <div style={{ fontSize: 12, fontWeight: 600 }}>Work together</div>
          <div style={{ fontSize: 9, color: hw.textFaint }}>
            the edits travel, the audio stays on your machine
          </div>
          <div style={{ flex: 1 }} />
          <button onClick={onClose} style={btn()}>Close</button>
        </div>

        <div style={{ padding: 14, display: 'flex', flexDirection: 'column', gap: 12 }}>
          {connected ? (
            <>
              <div style={{
                padding: 12, borderRadius: hw.radius.md,
                background: 'rgba(255,255,255,0.04)', border: `1px solid ${hw.border}`,
              }}>
                <div style={{ fontSize: 10, color: hw.textFaint }}>
                  {status?.hosting ? 'Your room. Read this out:' : 'In their room, code:'}
                </div>
                <div style={{
                  fontSize: 20, fontWeight: 700, letterSpacing: '0.08em',
                  fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace', marginTop: 4,
                }}>{status?.inviteCode || '—'}</div>
                <div style={{ fontSize: 10, color: hw.textFaint, marginTop: 6 }}>
                  Room {status?.roomId}. {status?.sent ?? 0} sent, {status?.received ?? 0} received.
                </div>
                <div style={{ marginTop: 10, display: 'flex', gap: 6, flexWrap: 'wrap' }} aria-live="polite">
                  {(status?.members ?? []).length <= 1 && (
                    <span style={{ fontSize: 11, color: hw.textFaint }}>
                      Waiting for the other person to come in.
                    </span>
                  )}
                  {(status?.members ?? []).length > 1 && (status?.members ?? []).map((name, i) => (
                    <span
                      key={name + i}
                      style={{
                        fontSize: 11, padding: '3px 8px', borderRadius: 999,
                        background: i === 0 ? hw.accent : 'rgba(255,255,255,0.08)',
                        color: i === 0 ? '#fff' : hw.textPrimary,
                      }}
                      title={i === 0 ? 'Opened the room' : 'Joined'}
                    >{name}</span>
                  ))}
                </div>
              </div>
              <div style={{ display: 'flex', gap: 8 }}>
                <button
                  onClick={() => {
                    invoke('request_project')
                      .then(() => useNotificationStore.getState().push('info', 'Asked them for the song', {
                        detail: 'It replaces what is open here when it arrives. The audio files do not travel with it.',
                      }))
                      .catch(e => setError(String(e)))
                  }}
                  title="Replaces the song open here with theirs"
                  style={btn()}
                >Get the song from them</button>
                <button onClick={() => void leave()} style={btn()}>Leave the room</button>
              </div>
            </>
          ) : (
            <>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                <div style={{ fontSize: 11, fontWeight: 600 }}>Open a room</div>
                <div style={{ fontSize: 10, color: hw.textFaint, lineHeight: 1.5 }}>
                  You get a code to read out. Opening a room is part of Pro; the person you
                  invite needs only a free account.
                </div>
                <button
                  onClick={() => void open()}
                  disabled={busy}
                  style={{ ...btn(), alignSelf: 'flex-start', background: hw.accent, color: '#fff' }}
                >{busy ? 'One moment…' : 'Open a room'}</button>
              </div>

              <div style={{ height: 1, background: hw.border }} />

              <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                <div style={{ fontSize: 11, fontWeight: 600 }}>Join theirs</div>
                <div style={{ display: 'flex', gap: 6 }}>
                  <input
                    value={room}
                    onChange={e => setRoom(e.target.value)}
                    placeholder="room"
                    aria-label="Room"
                    style={input()}
                  />
                  <input
                    value={code}
                    onChange={e => setCode(e.target.value.toUpperCase())}
                    placeholder="code"
                    aria-label="Invite code"
                    style={{ ...input(), letterSpacing: '0.08em' }}
                  />
                  <button onClick={() => void join()} disabled={busy || !code.trim()} style={btn()}>
                    Join
                  </button>
                </div>
              </div>
            </>
          )}

          {(error || status?.message) && (
            <div style={{ fontSize: 10, color: error ? hw.red : hw.textFaint, lineHeight: 1.5 }}>
              {error ?? status?.message}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

function btn(): React.CSSProperties {
  return {
    padding: '4px 10px', fontSize: 11, fontWeight: 600,
    background: 'rgba(255,255,255,0.08)', color: hw.textSecondary,
    border: 'none', borderRadius: hw.radius.sm, cursor: 'pointer', fontFamily: 'inherit',
  }
}

function input(): React.CSSProperties {
  return {
    flex: 1, minWidth: 0, padding: '4px 8px', fontSize: 11, fontFamily: 'inherit',
    background: 'rgba(255,255,255,0.04)', color: hw.textPrimary,
    border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
  }
}
