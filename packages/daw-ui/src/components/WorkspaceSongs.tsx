import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'
import { useProjectStore } from '../stores/projectStore'
import { useNotificationStore } from '../stores/notificationStore'
import { DialogFrame } from './ui/DialogFrame'

/**
 * Songs saved to Workspace.
 *
 * Each one is a folder with the project and the samples it uses, so
 * opening it here brings the whole thing down and it plays as it did
 * on the machine that saved it.
 */

interface CloudSong {
  name: string
  fileId: number
  folder: string
  updatedAt: string | null
  files: number
}

export function WorkspaceSongs({ onClose }: { onClose: () => void }) {
  const [songs, setSongs] = useState<CloudSong[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [opening, setOpening] = useState<number | null>(null)

  useEffect(() => {
    invoke<CloudSong[]>('list_workspace_songs')
      .then(setSongs)
      .catch(e => setError(String(e)))
  }, [])

  const open = async (song: CloudSong) => {
    setOpening(song.fileId)
    try {
      const path = await invoke<string>('open_from_workspace', { fileId: song.fileId })
      await useProjectStore.getState().loadProject(path)
      useNotificationStore.getState().push('info', `Opened "${song.name}" from Workspace`)
      onClose()
    } catch (e) {
      setError(String(e))
      setOpening(null)
    }
  }

  return (
    <DialogFrame
      title="Open from Workspace"
      subtitle="Songs saved with their samples"
      onClose={onClose}
      width={520}
    >
      <div style={{ overflowY: 'auto', padding: 8 }}>
        {error && <div style={{ padding: 10, fontSize: 12.5, color: hw.red }}>{error}</div>}
        {!error && songs === null && (
          <div style={{ padding: 16, fontSize: 12.5, color: hw.textFaint }}>Looking in your Workspace…</div>
        )}
        {songs && songs.length === 0 && (
          <div style={{ padding: 16, fontSize: 12.5, color: hw.textFaint, lineHeight: 1.6 }}>
            Nothing here yet. File &gt; Save to Workspace puts the open song here, samples and all.
          </div>
        )}
        {songs?.map(song => (
          <button
            key={song.fileId}
            onClick={() => void open(song)}
            disabled={opening !== null}
            style={{
              display: 'flex', width: '100%', alignItems: 'center', gap: 10,
              padding: '10px 12px', marginBottom: 4, textAlign: 'left',
              background: 'rgba(255,255,255,0.04)', border: `1px solid ${hw.border}`,
              borderRadius: hw.radius.md, color: hw.textPrimary, cursor: 'pointer',
              fontFamily: 'inherit',
            }}
          >
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 12, fontWeight: 600 }}>{song.name}</div>
              <div style={{ fontSize: 12, color: hw.textFaint }}>
                {song.files} {song.files === 1 ? 'file' : 'files'}
                {song.updatedAt ? ` · saved ${song.updatedAt.slice(0, 10)}` : ''}
              </div>
            </div>
            <span style={{ fontSize: 12, color: hw.textSecondary }}>
              {opening === song.fileId ? 'Bringing it down…' : 'Open'}
            </span>
          </button>
        ))}
      </div>
    </DialogFrame>
  )
}

