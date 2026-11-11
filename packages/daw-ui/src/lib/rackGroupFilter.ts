/**
 * The channel rack's filter, as FL has it: All, then the song's own groups
 * ("Audio", "Automation", "Unsorted" in a song from FL; "Unsorted" in a song
 * made here). A channel names its group by its place in the song's list
 * (its track's `rackGroup`).
 */
export type RackGroupFilter = 'all' | number

/** Whether a channel in group `rackGroup` shows under the filter. */
export function inRackGroup(filter: RackGroupFilter, rackGroup: number | null | undefined): boolean {
  return filter === 'all' || rackGroup === filter
}

export interface RackGroupButton {
  filter: RackGroupFilter
  label: string
  /** How many of the rack's channels it shows. */
  count: number
}

/**
 * The filter's buttons: All with every channel, then each of the song's
 * groups in the song's order with how many channels it holds. A group with
 * no channels still shows, as in FL.
 */
export function rackGroupButtons(
  groups: string[],
  channels: ReadonlyArray<{ rackGroup?: number | null }>,
): RackGroupButton[] {
  const counts = groups.map(() => 0)
  for (const c of channels) {
    if (c.rackGroup != null && c.rackGroup >= 0 && c.rackGroup < counts.length) counts[c.rackGroup] += 1
  }
  return [
    { filter: 'all', label: 'All', count: channels.length },
    ...groups.map((label, i) => ({ filter: i, label, count: counts[i] })),
  ]
}

/** The filter to keep once the song changes: a group it no longer has falls back to All. */
export function keptRackGroupFilter(filter: RackGroupFilter, groups: string[]): RackGroupFilter {
  return filter === 'all' || filter < groups.length ? filter : 'all'
}
