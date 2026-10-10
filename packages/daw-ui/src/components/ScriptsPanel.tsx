import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'
import { useNotificationStore } from '../stores/notificationStore'
import { DialogFrame } from './ui/DialogFrame'
import { btn } from './ui/dialogStyles'

/**
 * The user's own scripts.
 *
 * Some edits are a loop, not a gesture: forty hats on the off-beat,
 * every track down three dB, a clip moved a bar. A script writes them
 * once and runs them whenever. The language is Rhai, which reads like
 * plain Rust, and a run is one undo step however much it does.
 */

interface ScriptFile {
  name: string
  body: string
}

interface ScriptResult {
  output: string[]
  asked: number
  applied: number
}

const EXAMPLE = `// Four kicks on the beat.
for i in 0..4 {
    add_note("clip-id-here", beats(i), 36, 100, beats(0.25));
}
print("four kicks written");
`

export function ScriptsPanel({ onClose }: { onClose: () => void }) {
  const [scripts, setScripts] = useState<ScriptFile[]>([])
  const [name, setName] = useState('new script')
  const [body, setBody] = useState(EXAMPLE)
  const [log, setLog] = useState<string[]>([])
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const refresh = useCallback(async () => {
    try {
      setScripts(await invoke<ScriptFile[]>('list_scripts'))
    } catch { /* no engine in the browser preview */ }
  }, [])

  useEffect(() => { void refresh() }, [refresh])

  const check = useCallback(async () => {
    setError(null)
    setBusy(true)
    try {
      const result = await invoke<ScriptResult>('check_script', { body })
      setLog([...result.output, `${result.asked} ${result.asked === 1 ? 'command' : 'commands'}, nothing applied`])
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }, [body])

  const run = useCallback(async () => {
    setError(null)
    setBusy(true)
    try {
      const result = await invoke<ScriptResult>('run_script', { body })
      const skipped = result.asked - result.applied
      setLog([
        ...result.output,
        `${result.applied} of ${result.asked} applied`
          + (skipped > 0 ? `, ${skipped} skipped because what they named was not there` : ''),
      ])
    } catch (e) {
      setError(String(e))
      useNotificationStore.getState().push('warning', 'The script stopped', { detail: String(e) })
    } finally {
      setBusy(false)
    }
  }, [body])

  const save = useCallback(async () => {
    try {
      await invoke('save_script', { name, body })
      await refresh()
      useNotificationStore.getState().push('info', `Saved "${name}"`)
    } catch (e) {
      setError(String(e))
    }
  }, [name, body, refresh])

  const remove = useCallback(async (which: string) => {
    try {
      await invoke('delete_script', { name: which })
      await refresh()
    } catch (e) {
      setError(String(e))
    }
  }, [refresh])

  return (
    <DialogFrame
      title="Scripts"
      subtitle="A run is one undo step, however much it does"
      onClose={onClose}
      width={820}
    >
      <div style={{ display: 'flex', minHeight: 0, flex: 1 }}>
        <div style={{
          width: 190, borderRight: `1px solid ${hw.border}`,
          overflowY: 'auto', padding: 8, display: 'flex', flexDirection: 'column', gap: 4,
        }}>
          {scripts.length === 0 && (
            <div style={{ fontSize: 12, color: hw.textFaint, padding: 6 }}>
              Nothing saved yet.
            </div>
          )}
          {scripts.map(script => (
            <div key={script.name} style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
              <button
                onClick={() => { setName(script.name); setBody(script.body); setLog([]); setError(null) }}
                style={{
                  ...btn(), flex: 1, textAlign: 'left',
                  background: script.name === name ? hw.accent : 'rgba(255,255,255,0.06)',
                  color: script.name === name ? '#fff' : hw.textSecondary,
                }}
              >{script.name}</button>
              <button
                onClick={() => void remove(script.name)}
                title={`Delete ${script.name}`}
                style={{ ...btn(), padding: '3px 6px' }}
              >×</button>
            </div>
          ))}
        </div>

        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, padding: 10, gap: 8 }}>
          <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            <input
              value={name}
              onChange={e => setName(e.target.value)}
              aria-label="Script name"
              style={{
                flex: 1, padding: '4px 8px', fontSize: 12.5, fontFamily: 'inherit',
                background: 'rgba(255,255,255,0.04)', color: hw.textPrimary,
                border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
              }}
            />
            <button onClick={() => void save()} style={btn()}>Save</button>
            <button onClick={() => void check()} disabled={busy} style={btn()}>Check</button>
            <button
              onClick={() => void run()}
              disabled={busy}
              style={{ ...btn(), background: hw.accent, color: '#fff' }}
            >Run</button>
          </div>

          <textarea
            value={body}
            onChange={e => setBody(e.target.value)}
            spellCheck={false}
            aria-label="Script"
            style={{
              flex: 1, minHeight: 240, resize: 'vertical',
              padding: 10, fontSize: 12, lineHeight: 1.5,
              fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
              background: 'rgba(0,0,0,0.35)', color: hw.textPrimary,
              border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
              whiteSpace: 'pre', overflowWrap: 'normal', overflowX: 'auto',
            }}
          />

          <div style={{
            minHeight: 54, maxHeight: 120, overflowY: 'auto',
            padding: 8, fontSize: 12.5, lineHeight: 1.5,
            fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
            background: 'rgba(0,0,0,0.25)', borderRadius: hw.radius.sm,
            color: error ? hw.red : hw.textSecondary,
            border: `1px solid ${hw.border}`,
          }}>
            {error ?? (log.length > 0 ? log.join('\n') : 'Check reads the script without changing anything.')}
          </div>

          <details style={{ fontSize: 12, color: hw.textFaint }}>
            <summary style={{ cursor: 'pointer' }}>What a script can call</summary>
            <pre style={{ margin: '6px 0 0', whiteSpace: 'pre-wrap', fontSize: 12 }}>{
`play()  stop()  seek(tick)
set_volume(track_id, db)   set_pan(track_id, -1..1)
set_muted(track_id, true)  set_master_volume(db)
add_note(clip_id, tick, pitch, velocity, length)
delete_note(clip_id, tick, pitch)
move_clip(clip_id, tick)   delete_clip(clip_id)
beats(n)  bars(n)   ticks, so you can write beats(2)
print(text)`
            }</pre>
          </details>
        </div>
      </div>
    </DialogFrame>
  )
}

