import { memo, useEffect } from 'react'
import { createPortal } from 'react-dom'
import { useMixerSelectionStore } from '../../../stores/mixerSelectionStore'
import { useTrackStore } from '../../../stores/trackStore'
import { useColorPickerStore } from '../../../stores/colorPickerStore'
import { insertsByNumber } from '../../../lib/mixerNumbers'
import { withNewInsert } from '../../../lib/insertActions'
import { t, useLanguage } from '../../../i18n'

/**
 * A strip's right-click menu, as mockup M2 draws it: the insert's number,
 * Rename (F2) and Colour. On an empty number, Colour makes the insert first,
 * one undo step with the colour.
 */
export const InsertMenu = memo(function InsertMenu() {
  useLanguage()
  const menu = useMixerSelectionStore((s) => s.stripMenu)
  const trackId = useTrackStore((s) => (menu ? insertsByNumber(s.tracks).get(menu.number)?.id ?? null : null))
  const color = useTrackStore((s) => (trackId ? s.tracksById[trackId]?.color ?? null : null))

  useEffect(() => {
    if (!menu) return
    const close = (e: MouseEvent) => {
      if ((e.target as HTMLElement).closest('.mx-insert-menu')) return
      useMixerSelectionStore.getState().openStripMenu(null)
    }
    const escape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') useMixerSelectionStore.getState().openStripMenu(null)
    }
    window.addEventListener('mousedown', close)
    window.addEventListener('keydown', escape)
    return () => {
      window.removeEventListener('mousedown', close)
      window.removeEventListener('keydown', escape)
    }
  }, [menu])

  if (!menu) return null
  const { number, x, y } = menu
  const rename = () => useMixerSelectionStore.getState().setRenamingNumber(number)
  const recolour = (e: React.MouseEvent) => {
    const anchor = (e.currentTarget as HTMLElement).getBoundingClientRect()
    useMixerSelectionStore.getState().openStripMenu(null)
    useColorPickerStore.getState().open({
      anchor,
      current: color,
      title: `${t('Insert')} ${number}`,
      onPick: (c) => {
        const set = (id: string) => useTrackStore.getState().setTrackColor(id, c)
        if (trackId) void set(trackId)
        else void withNewInsert(number, `Colour insert ${number}`, async (id) => {
          await set(id)
          useMixerSelectionStore.getState().selectTrack(id)
        }).catch(() => {})
      },
    })
  }
  return createPortal(
    <div
      className="mx-insert-menu"
      role="menu"
      style={{ left: Math.min(x, window.innerWidth - 220), top: Math.min(y, window.innerHeight - 110) }}
      onContextMenu={(e) => e.preventDefault()}
    >
      <div className="mx-insert-menu-title">{`${t('INSERT')} ${number}`}</div>
      <button type="button" role="menuitem" onClick={rename}>
        {t('Rename')}<small>F2</small>
      </button>
      <button type="button" role="menuitem" onClick={recolour}>
        {t('Colour')}
      </button>
    </div>,
    document.body,
  )
})
