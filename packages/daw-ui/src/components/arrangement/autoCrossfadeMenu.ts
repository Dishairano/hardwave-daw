import type { MenuItem } from '../HwTopMenu'
import { useTrackStore } from '../../stores/trackStore'
import { useNotificationStore } from '../../stores/notificationStore'

/** Edit > Auto-crossfade overlaps: every overlap between two audio clips
 *  of one channel on a row gets a crossfade, as one undo step, and the
 *  notification says how many. The action was there with nothing to start
 *  it, while the help pointed to this menu item. */
export function autoCrossfadeMenuItem(): MenuItem {
  return {
    label: 'Auto-crossfade overlaps',
    action: () => { void crossfadeEveryOverlap() },
  }
}

export async function crossfadeEveryOverlap(): Promise<number> {
  const n = await useTrackStore.getState().autoCrossfadeOverlaps()
  const push = useNotificationStore.getState().push
  if (n === 0) push('info', 'No overlapping audio clips to crossfade')
  else push('info', `Crossfaded ${n} overlap${n === 1 ? '' : 's'}`)
  return n
}
