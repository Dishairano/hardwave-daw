import { memo } from 'react'
import { ChannelStrip } from './ChannelStrip'
import { useTrackStore } from '../../../stores/trackStore'

export interface MasterStripProps {
  selected: boolean
  onSelect: (trackId: string) => void
}

/**
 * Master strip — same shape as a regular ChannelStrip, just pinned to the
 * far-left grid column with the red `.master` modifier theme. We re-use
 * ChannelStrip wholesale to guarantee visual + behavior parity (the user
 * spec is "master must look the same as inserts").
 *
 * If the project somehow has no Master track (shouldn't happen in a real
 * session, but on first boot before the engine seeds it) we render a
 * placeholder strip so the layout doesn't jump.
 */
export const MasterStrip = memo(function MasterStrip(props: MasterStripProps) {
  const masterId = useTrackStore((s) => s.tracks.find((t) => t.kind === 'Master')?.id)
  if (!masterId) {
    return <div className="mx-strip master mx-strip-placeholder" />
  }
  return (
    <ChannelStrip
      trackId={masterId}
      index="M"
      selected={props.selected}
      onSelect={props.onSelect}
      variant="master"
    />
  )
})
