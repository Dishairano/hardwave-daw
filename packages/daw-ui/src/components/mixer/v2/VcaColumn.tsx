import { memo, useEffect } from 'react'
import { VcaStrip } from './VcaStrip'
import { useVcaStore } from '../../../stores/vcaStore'

/**
 * The VCA faders, in a column of their own between the channels and the FX
 * rack. A song starts with 500 channels, so VCAs placed after them would sit
 * at the far end of the mixer; here they stay in view while the channels
 * scroll past.
 */
export const VcaColumn = memo(function VcaColumn() {
  const vcas = useVcaStore(s => s.vcas)
  useEffect(() => { void useVcaStore.getState().fetch() }, [])
  return (
    <div className="mx-vca-col">
      {vcas.map((vca, i) => (
        <div key={vca.id} className="mx-vca-slot">
          <VcaStrip vca={vca} index={i + 1} />
        </div>
      ))}
      <button
        className="mx-add-strip mx-add-vca"
        title="Add a VCA: one fader that moves a group of tracks together"
        onClick={() => { void useVcaStore.getState().add() }}
      >
        <span>+</span>
        <small>VCA</small>
      </button>
    </div>
  )
})
