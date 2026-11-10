import { memo, useEffect, useRef, useState } from 'react'
import { useMixerSelectionStore } from '../../../stores/mixerSelectionStore'
import { useTrackStore } from '../../../stores/trackStore'
import { insertName } from '../../../lib/mixerNumbers'
import { makeInsert } from '../../../lib/insertActions'
import { t, useLanguage } from '../../../i18n'

/**
 * The name under a strip's number: the insert's name, or "Insert N" as FL
 * calls one nobody named. A double-click, F2 on the selected strip, or
 * Rename in its right-click menu edits it in place; Enter keeps the new
 * name, Escape keeps the old one. Naming an empty number makes it an
 * insert with that name. The V2 mixer could not rename a strip before.
 */
export const StripName = memo(function StripName({ number, trackId, name }: {
  number: number
  /** null for an empty number. */
  trackId: string | null
  name: string
}) {
  useLanguage()
  const renaming = useMixerSelectionStore((s) => s.renamingNumber === number)
  const label = insertName(number, { name })
  if (renaming && number !== 0) return <NameInput number={number} trackId={trackId} initial={trackId ? label : ''} />
  return (
    <div
      className="mx-s-name"
      title={number === 0 ? label : `${number} ${label}. ${t('Double-click to rename')}`}
      onDoubleClick={(e) => {
        if (number === 0) return
        e.stopPropagation()
        useMixerSelectionStore.getState().setRenamingNumber(number)
      }}
    >
      {label}
    </div>
  )
})

function NameInput({ number, trackId, initial }: { number: number; trackId: string | null; initial: string }) {
  const [value, setValue] = useState(initial)
  const done = useRef(false)
  const ref = useRef<HTMLInputElement>(null)
  useEffect(() => {
    ref.current?.focus()
    ref.current?.select()
  }, [])
  const finish = (keep: boolean) => {
    if (done.current) return
    done.current = true
    useMixerSelectionStore.getState().setRenamingNumber(null)
    const next = value.trim()
    if (!keep || !next || next === initial) return
    if (trackId) void useTrackStore.getState().renameTrack(trackId, next)
    else void makeInsert(number, next).then((id) => useMixerSelectionStore.getState().selectTrack(id)).catch(() => {})
  }
  return (
    <input
      ref={ref}
      className="mx-s-name-input"
      value={value}
      placeholder={`Insert ${number}`}
      aria-label={t('Insert name')}
      onChange={(e) => setValue(e.target.value)}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        e.stopPropagation()
        if (e.key === 'Enter') finish(true)
        else if (e.key === 'Escape') finish(false)
      }}
      onBlur={() => finish(true)}
    />
  )
}
