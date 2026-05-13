import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'

/**
 * Project Info dialog — FL Studio's File → Project info page.
 * Title / Author / Genre / Info / URL plus the "Show on Open" splash
 * toggle and a cumulative working-time counter the user can reset.
 *
 * Metadata round-trips through `get_project_meta` / `set_project_meta`.
 * The counter ticks via `tick_project_working_time` on a 30s cadence
 * driven by App.tsx whenever the window has focus.
 */

interface ProjectInfoMeta {
  title: string
  author: string
  genre: string
  info: string
  url: string
  show_on_open: boolean
  working_time_seconds: number
}

interface Props {
  onClose: () => void
}

const GENRE_SUGGESTIONS = [
  'Hardstyle', 'Hardcore', 'Techno', 'House', 'Drum & Bass', 'Trance',
  'Dubstep', 'Hip-Hop', 'Trap', 'Ambient', 'Cinematic', 'Pop', 'Rock',
]

export function ProjectInfoDialog({ onClose }: Props) {
  const [meta, setMeta] = useState<ProjectInfoMeta | null>(null)
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    void (async () => {
      try {
        setMeta(await invoke<ProjectInfoMeta>('get_project_meta'))
      } catch (err) {
        console.warn('get_project_meta:', err)
      }
    })()
  }, [])

  if (!meta) return null

  const save = async () => {
    setSaving(true)
    try {
      await invoke('set_project_meta', { meta })
      onClose()
    } catch (err) {
      console.warn('set_project_meta:', err)
    } finally {
      setSaving(false)
    }
  }

  const resetWorkingTime = async () => {
    try {
      await invoke('reset_project_working_time')
      setMeta({ ...meta, working_time_seconds: 0 })
    } catch (err) {
      console.warn('reset_project_working_time:', err)
    }
  }

  const fmtTime = (s: number) => {
    const h = Math.floor(s / 3600)
    const m = Math.floor((s % 3600) / 60)
    const sec = s % 60
    return `${h}h ${m}m ${sec}s`
  }

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed', inset: 0, zIndex: 200,
        background: 'rgba(0,0,0,0.6)', backdropFilter: 'blur(4px)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        padding: 24,
      }}
    >
      <div
        onClick={e => e.stopPropagation()}
        style={{
          width: 480, maxHeight: '85vh',
          background: 'rgba(12,12,16,0.98)',
          border: `1px solid ${hw.borderLight}`,
          borderRadius: hw.radius.lg,
          boxShadow: '0 20px 60px rgba(0,0,0,0.7)',
          overflow: 'hidden',
          display: 'flex', flexDirection: 'column',
        }}
      >
        <div style={{
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          padding: '12px 16px',
          background: 'rgba(255,255,255,0.03)',
          borderBottom: `1px solid ${hw.border}`,
        }}>
          <span style={{ fontSize: 13, fontWeight: 700, color: hw.accent, letterSpacing: 0.5 }}>
            PROJECT INFO
          </span>
          <button
            onClick={onClose}
            style={{
              background: 'transparent', border: 0, color: hw.textFaint,
              cursor: 'pointer', fontSize: 16, padding: 4,
            }}
            aria-label="Close"
          >×</button>
        </div>

        <div style={{ padding: 16, overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: 12 }}>
          <Field label="Title">
            <input
              value={meta.title}
              onChange={e => setMeta({ ...meta, title: e.target.value })}
              placeholder="Untitled"
              style={inputStyle}
            />
          </Field>

          <Field label="Author">
            <input
              value={meta.author}
              onChange={e => setMeta({ ...meta, author: e.target.value })}
              placeholder="Your name / artist alias"
              style={inputStyle}
            />
          </Field>

          <Field label="Genre">
            <input
              value={meta.genre}
              onChange={e => setMeta({ ...meta, genre: e.target.value })}
              list="hw-genre-suggestions"
              placeholder="e.g. Hardstyle, Techno, …"
              style={inputStyle}
            />
            <datalist id="hw-genre-suggestions">
              {GENRE_SUGGESTIONS.map(g => <option key={g} value={g} />)}
            </datalist>
          </Field>

          <Field label="Info / Description">
            <textarea
              value={meta.info}
              onChange={e => setMeta({ ...meta, info: e.target.value })}
              placeholder="Notes for collaborators or your future self"
              rows={4}
              style={{ ...inputStyle, resize: 'vertical', minHeight: 70, fontFamily: 'inherit' }}
            />
          </Field>

          <Field label="URL">
            <input
              value={meta.url}
              onChange={e => setMeta({ ...meta, url: e.target.value })}
              placeholder="https:// or mailto:"
              style={inputStyle}
            />
          </Field>

          <label style={{
            display: 'flex', alignItems: 'center', gap: 10,
            padding: '8px 10px',
            background: 'rgba(0,0,0,0.25)',
            border: `1px solid ${meta.show_on_open ? hw.accent : hw.borderDark}`,
            borderRadius: hw.radius.sm, cursor: 'pointer', fontSize: 12,
          }}>
            <input
              type="checkbox"
              checked={meta.show_on_open}
              onChange={e => setMeta({ ...meta, show_on_open: e.target.checked })}
            />
            <div>
              <div style={{ color: hw.textPrimary }}>Show on open</div>
              <div style={{ color: hw.textFaint, fontSize: 10, marginTop: 2 }}>
                Display the Title + Info + URL splash whenever this project is loaded.
              </div>
            </div>
          </label>

          <div style={{
            display: 'flex', alignItems: 'center', justifyContent: 'space-between',
            padding: '8px 10px',
            background: 'rgba(0,0,0,0.25)',
            border: `1px solid ${hw.borderDark}`,
            borderRadius: hw.radius.sm, fontSize: 12,
          }}>
            <div>
              <div style={{ color: hw.textPrimary }}>Working time</div>
              <div style={{ color: hw.textFaint, fontFamily: 'monospace', fontSize: 11, marginTop: 2 }}>
                {fmtTime(meta.working_time_seconds)}
              </div>
            </div>
            <button
              onClick={resetWorkingTime}
              style={{
                padding: '5px 12px', fontSize: 10, fontWeight: 600,
                background: 'rgba(255,255,255,0.08)', color: hw.textSecondary,
                border: `1px solid ${hw.borderDark}`, borderRadius: hw.radius.sm,
                cursor: 'pointer', fontFamily: 'inherit',
              }}
            >Reset</button>
          </div>
        </div>

        <div style={{
          padding: '12px 16px', borderTop: `1px solid ${hw.border}`,
          background: 'rgba(255,255,255,0.02)',
          display: 'flex', gap: 8, justifyContent: 'flex-end',
        }}>
          <button
            onClick={onClose}
            style={{
              padding: '7px 14px', fontSize: 11, fontWeight: 600,
              background: 'transparent', color: hw.textSecondary,
              border: `1px solid ${hw.borderDark}`, borderRadius: hw.radius.sm,
              cursor: 'pointer', fontFamily: 'inherit',
            }}
          >Cancel</button>
          <button
            onClick={save}
            disabled={saving}
            style={{
              padding: '7px 14px', fontSize: 11, fontWeight: 600,
              background: hw.accent, color: '#fff',
              border: `1px solid ${hw.accent}`, borderRadius: hw.radius.sm,
              cursor: saving ? 'wait' : 'pointer', fontFamily: 'inherit',
            }}
          >{saving ? 'Saving…' : 'Save'}</button>
        </div>
      </div>
    </div>
  )
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
      <label style={{ fontSize: 10, color: hw.textMuted, letterSpacing: 0.4, textTransform: 'uppercase' }}>{label}</label>
      {children}
    </div>
  )
}

const inputStyle: React.CSSProperties = {
  padding: '7px 10px',
  background: 'rgba(0,0,0,0.35)',
  border: `1px solid ${hw.borderDark}`,
  borderRadius: hw.radius.sm,
  color: hw.textPrimary,
  fontSize: 12,
  fontFamily: 'inherit',
  outline: 'none',
}
