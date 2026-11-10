import { invokeOrToast } from '../api/invoke'
import { useTrackStore } from '../stores/trackStore'
import { useProjectStore } from '../stores/projectStore'
import { useHistoryStore } from '../stores/historyStore'

/**
 * What the mixer, the rack and the routing panel do with insert numbers.
 * Each is one undo step in the engine (make_insert, route_to_insert), or
 * one group of steps when an empty number is made and then used.
 */

/** Send channels to insert `number` (0 the master). An empty number becomes
 *  an insert. Returns the id of an insert made for it, or null. */
export async function routeToInsert(trackIds: string[], number: number, label?: string): Promise<string | null> {
  if (!trackIds.length) return null
  const since = Date.now()
  const made = await invokeOrToast<string | null>('route_to_insert', { trackIds, number }, {
    message: 'Could not change where the channel plays',
    retry: false,
  })
  useProjectStore.getState().markDirty()
  if (label) useHistoryStore.getState().push(label, since)
  await useTrackStore.getState().fetchTracks()
  return made
}

/** Make insert `number` a track (an empty number in the mixer), with a name
 *  or "Insert N". Returns its id. */
export async function makeInsert(number: number, name?: string): Promise<string> {
  const id = await invokeOrToast<string>('make_insert', { number, name: name ?? null }, {
    message: 'Could not make the insert',
    retry: false,
  })
  useProjectStore.getState().markDirty()
  await useTrackStore.getState().fetchTracks()
  return id
}

/** Make an empty number an insert and do `then` with it, as one undo step
 *  named `label`: an effect, a send or a name put on an empty insert. */
export async function withNewInsert<T>(number: number, label: string, then: (id: string) => Promise<T>): Promise<T> {
  const store = useTrackStore.getState()
  await store.beginHistoryGroup()
  try {
    const id = await makeInsert(number)
    return await then(id)
  } finally {
    await store.endHistoryGroup(label)
  }
}
