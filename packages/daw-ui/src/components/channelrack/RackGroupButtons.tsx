import { hw } from '../../theme'
import { t } from '../../i18n'
import type { RackGroupButton, RackGroupFilter as Filter } from '../../lib/rackGroupFilter'

/**
 * The channel rack's filter, as FL has it: All, then the song's groups, each
 * with how many channels it holds. Replaces the All / Audio / MIDI buttons;
 * the kind still shows on every channel.
 */
export function RackGroupButtons({ buttons, filter, onPick }: {
  buttons: RackGroupButton[]
  filter: Filter
  onPick: (filter: Filter) => void
}) {
  return (
    <div data-testid="rack-group-filter" style={{ display: 'flex', gap: 2, alignItems: 'center', minWidth: 0, overflow: 'hidden' }}>
      {buttons.map((b) => {
        const on = b.filter === filter
        return (
          <button
            key={String(b.filter)}
            data-testid={`rack-group-${b.filter}`}
            onClick={() => onPick(b.filter)}
            title={b.filter === 'all' ? t('Show every channel') : b.label}
            style={{
              height: 18, display: 'flex', alignItems: 'center', gap: 5, flexShrink: 0,
              padding: '0 9px', borderRadius: 6, cursor: 'pointer', fontSize: 11,
              whiteSpace: 'nowrap', maxWidth: 140,
              color: on ? hw.textBright : hw.textSecondary,
              background: on ? hw.accentDim : 'transparent',
              border: `1px solid ${on ? hw.accentGlow : 'transparent'}`,
            }}
          >
            <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>
              {b.filter === 'all' ? t('All') : b.label}
            </span>
            <small style={{ fontSize: 9.5, fontWeight: 600, fontFamily: hw.font.mono, color: on ? hw.accent : hw.textFaint }}>
              {b.count}
            </small>
          </button>
        )
      })}
    </div>
  )
}
