import { memo, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Fader } from '../../primitives/Fader'
import { DbScale } from './DbScale'
import { useVcaStore, type Vca } from '../../../stores/vcaStore'
import { useTrackStore } from '../../../stores/trackStore'
import { t, tf, useLanguage } from '../../../i18n'

/**
 * One VCA fader in the mixer: an amber cap, a mute, a fader that moves every
 * member by the same amount, and the list of its members.
 */
export const VcaStrip = memo(function VcaStrip({ vca, index }: { vca: Vca; index: number }) {
  useLanguage()
  const store = useVcaStore.getState()
  const [membersAt, setMembersAt] = useState<{ x: number; y: number } | null>(null)
  const count = vca.members.length

  return (
    <div className="mx-strip mx-vca-strip">
      <div className="mx-s-num">
        <span className="mx-s-num-idx mx-vca-cap">V{index}</span>
      </div>
      <div
        className="mx-s-name"
        title={t('Double-click to rename')}
        onDoubleClick={() => {
          const name = window.prompt(t('VCA name:'), vca.name)?.trim()
          if (name && name !== vca.name) void store.rename(vca.id, name)
        }}
      >
        {vca.name}
      </div>
      <div className="mx-s-pills">
        <button
          className={'mx-pill mx-pill-m' + (vca.muted ? ' on' : '')}
          onClick={() => { void store.setMuted(vca.id, !vca.muted) }}
          title={vca.muted ? t('Unmute every track in this VCA') : t('Mute every track in this VCA (their own mute buttons stay as they are)')}
        >
          M
        </button>
      </div>
      <div className="mx-vca-count">{count === 1 ? t('1 track') : tf('{n} tracks', { n: count })}</div>
      <div className="mx-s-fader-meter">
        <div className="mx-fader-col">
          <Fader
            valueDb={vca.gain_db}
            onChange={(db) => store.setGainLive(vca.id, db)}
            title={t('Moves every member by the same amount')}
          />
        </div>
        <DbScale />
      </div>
      <div className="mx-s-db">{vca.gain_db <= -60 ? '-∞' : vca.gain_db.toFixed(1)} dB</div>
      <button
        type="button"
        className="mx-vca-edit"
        onClick={(e) => {
          const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
          setMembersAt({ x: r.left, y: r.top })
        }}
      >
        {t('Members…')}
      </button>
      {membersAt && (
        <VcaMembers vca={vca} at={membersAt} onClose={() => setMembersAt(null)} />
      )}
    </div>
  )
})

function VcaMembers({ vca, at, onClose }: { vca: Vca; at: { x: number; y: number }; onClose: () => void }) {
  useLanguage()
  const tracks = useTrackStore(s => s.tracks)
  const store = useVcaStore.getState()
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const close = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose()
    }
    const escape = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('mousedown', close)
    window.addEventListener('keydown', escape)
    return () => {
      window.removeEventListener('mousedown', close)
      window.removeEventListener('keydown', escape)
    }
  }, [onClose])
  const candidates = tracks.filter(t => t.kind !== 'Master' && t.kind !== 'Automation')
  const toggle = (id: string) => {
    const next = vca.members.includes(id) ? vca.members.filter(m => m !== id) : [...vca.members, id]
    void store.setMembers(vca.id, next)
  }
  const height = Math.min(420, 70 + candidates.length * 26)
  return createPortal(
    <div
      ref={ref}
      className="mx-vca-pop"
      style={{ left: Math.min(at.x, window.innerWidth - 250), top: Math.max(8, Math.min(at.y - height, window.innerHeight - height - 8)), maxHeight: height }}
    >
      <h4>{tf('{name}: members', { name: vca.name })}</h4>
      <div className="mx-vca-list">
        {candidates.map(t => (
          <label key={t.id}>
            <input type="checkbox" checked={vca.members.includes(t.id)} onChange={() => toggle(t.id)} />
            {t.name}
          </label>
        ))}
      </div>
      <button
        type="button"
        className="mx-vca-delete"
        onClick={() => { onClose(); void store.remove(vca.id) }}
      >
        {t('Delete this VCA')}
      </button>
    </div>,
    document.body,
  )
}
