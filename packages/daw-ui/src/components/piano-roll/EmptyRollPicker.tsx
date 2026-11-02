import { useEffect, useMemo, useState } from 'react'
import { useTrackStore } from '../../stores/trackStore'
import { usePluginCatalogStore } from '../../stores/pluginCatalogStore'
import { useNotificationStore } from '../../stores/notificationStore'
import { channelSound, SAMPLER_ID, TRIPLE_OSC_ID } from '../../stores/instrumentChannels'
import { addInstrumentToPianoRoll, openChannelInPianoRoll, type NewInstrument } from '../../stores/pianoRollBinding'
import { t, tf, useLanguage } from '../../i18n'

/**
 * What the piano roll shows when it has nothing to edit: the song's
 * instrument channels and the instruments that can be added, right in the
 * grid. Picking one opens the roll on it, ready to draw. A click anywhere
 * else in the grid adds Hardwave 3xOSC with the note (see PianoRoll).
 */

interface BuiltIn {
  key: string
  name: string
  note: string
  color: string
  add: NewInstrument
}

/** Hardwave's own instruments, in the order the picker offers them. These
 *  are the ones the plug-in factory makes as instruments, and the kick
 *  synth, which is a channel's own voice rather than a plug-in. */
export const BUILT_IN_INSTRUMENTS: BuiltIn[] = [
  { key: TRIPLE_OSC_ID, name: 'Hardwave 3xOSC', note: 'three oscillators: leads, plucks, basses', color: '#3B82F6', add: { pluginId: TRIPLE_OSC_ID, name: 'Hardwave 3xOSC' } },
  { key: 'hardwave.native.wavetable', name: 'Hardwave Wavetable', note: 'five wavetable banks', color: '#06B6D4', add: { pluginId: 'hardwave.native.wavetable', name: 'Hardwave Wavetable' } },
  { key: 'hardwave.native.fm', name: 'Hardwave FM', note: 'eight-voice FM synth', color: '#EC4899', add: { pluginId: 'hardwave.native.fm', name: 'Hardwave FM' } },
  { key: 'hardwave.native.noise', name: 'Hardwave Noise', note: 'white, pink or brown noise', color: '#A855F7', add: { pluginId: 'hardwave.native.noise', name: 'Hardwave Noise' } },
  { key: SAMPLER_ID, name: 'Hardwave Sampler', note: 'one sample across the keys', color: '#10B981', add: { pluginId: SAMPLER_ID, name: 'Hardwave Sampler' } },
  { key: 'kick_synth', name: 'Kick synth', note: 'the four-layer kick', color: '#DC2626', add: 'kick_synth' },
]

const card: React.CSSProperties = {
  position: 'absolute', left: '50%', top: '50%', transform: 'translate(-50%, -50%)',
  width: 620, maxWidth: 'calc(100% - 24px)', maxHeight: 'calc(100% - 24px)', overflowY: 'auto',
  padding: '20px 22px 16px', boxSizing: 'border-box', zIndex: 5,
  background: '#111116', border: '1px solid rgba(255,255,255,0.08)', borderRadius: 12,
  boxShadow: '0 0 40px rgba(220,38,38,0.14), 0 26px 72px rgba(0,0,0,0.7)',
  color: '#fafafa', textAlign: 'left',
}
const eyebrow: React.CSSProperties = {
  fontSize: 11, color: '#71717a', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 10,
}

function Row({ color, name, note, onPick, disabled, testId }: {
  color: string; name: string; note?: string | null; onPick: () => void; disabled?: boolean; testId?: string
}) {
  const [hover, setHover] = useState(false)
  return (
    <button
      type="button"
      data-testid={testId}
      disabled={disabled}
      onClick={onPick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        display: 'flex', alignItems: 'center', gap: 9, width: '100%', padding: '6px 9px',
        borderRadius: 7, fontSize: 12.5, color: '#fafafa', textAlign: 'left', cursor: disabled ? 'default' : 'pointer',
        background: hover && !disabled ? 'rgba(220,38,38,0.12)' : 'transparent',
        border: `1px solid ${hover && !disabled ? 'rgba(220,38,38,0.35)' : 'transparent'}`,
        opacity: disabled ? 0.6 : 1, fontFamily: 'inherit',
      }}
    >
      <i style={{ width: 9, height: 9, borderRadius: 2, flex: 'none', background: color }} />
      <b style={{ fontWeight: 600, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', minWidth: 0, flexShrink: 0, maxWidth: '70%' }}>{name}</b>
      {note && (
        <small style={{
          marginLeft: 'auto', paddingLeft: 8, color: '#8b8b93', fontSize: 10.5,
          whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', minWidth: 0,
        }}>{note}</small>
      )}
    </button>
  )
}

export function EmptyRollPicker() {
  useLanguage()
  const tracks = useTrackStore(s => s.tracks)
  const plugins = usePluginCatalogStore(s => s.plugins)
  useEffect(() => { void usePluginCatalogStore.getState().ensureLoaded() }, [])
  const [busy, setBusy] = useState(false)
  const [showOthers, setShowOthers] = useState(false)

  // The song's instrument channels, as the channel rack lists them.
  const channels = useMemo(() => tracks.filter(tr => tr.kind === 'Midi'), [tracks])
  // Instruments the plug-in scan found in VST3 and CLAP folders.
  const others = useMemo(
    () => plugins
      .filter(p => p.category === 'Instrument' && !p.id.startsWith('hardwave.native.'))
      .sort((a, b) => a.name.localeCompare(b.name)),
    [plugins],
  )

  const run = (what: () => Promise<unknown>) => {
    if (busy) return
    setBusy(true)
    what()
      .catch(e => useNotificationStore.getState().push('error', t('The instrument was not added'), { detail: String(e) }))
      .finally(() => setBusy(false))
  }

  return (
    <div data-testid="empty-roll-picker" style={card} onMouseDown={e => e.stopPropagation()}>
      <h4 style={{ margin: '0 0 4px', fontSize: 17, fontWeight: 700 }}>{t('Pick what to play')}</h4>
      <p style={{ margin: '0 0 14px', fontSize: 12.5, color: '#b6b6c2', lineHeight: 1.55 }}>
        {t('Nothing to draw in yet. Choose a channel in this song or add an instrument, and the piano roll opens on it, ready to draw.')}
      </p>
      <div style={{ display: 'grid', gridTemplateColumns: '200px 1fr', gap: 18 }}>
        <div style={{ minWidth: 0 }}>
          <div style={eyebrow}>{t('In this song')}</div>
          {channels.map(ch => (
            <Row
              key={ch.id}
              testId="roll-channel"
              color={ch.color}
              name={ch.name}
              note={channelSound(ch, true)}
              disabled={busy}
              onPick={() => run(() => openChannelInPianoRoll(ch.id))}
            />
          ))}
          {channels.length === 0 && (
            <div style={{ fontSize: 11.5, color: '#71717a', padding: '6px 9px' }}>{t('No instrument channels yet.')}</div>
          )}
        </div>
        <div style={{ minWidth: 0 }}>
          <div style={eyebrow}>{showOthers ? t('VST3 and CLAP instruments') : t('Add an instrument')}</div>
          {!showOthers && BUILT_IN_INSTRUMENTS.map(b => (
            <Row
              key={b.key}
              testId="roll-add"
              color={b.color}
              name={b.key === 'kick_synth' ? t(b.name) : b.name}
              note={t(b.note)}
              disabled={busy}
              onPick={() => run(() => addInstrumentToPianoRoll(b.add))}
            />
          ))}
          {!showOthers && (
            <Row
              testId="roll-others"
              color="#71717a"
              name={t('VST3 and CLAP instruments')}
              note={others.length > 0 ? tf('{n} found ›', { n: others.length }) : t('none found')}
              disabled={busy || others.length === 0}
              onPick={() => setShowOthers(true)}
            />
          )}
          {showOthers && (
            <>
              <Row color="#71717a" name={t('‹ Hardwave instruments')} onPick={() => setShowOthers(false)} />
              {others.map(p => (
                <Row
                  key={p.id}
                  testId="roll-add-other"
                  color="#71717a"
                  name={p.name}
                  note={p.vendor}
                  disabled={busy}
                  onPick={() => run(() => addInstrumentToPianoRoll({ pluginId: p.id, name: p.name }))}
                />
              ))}
            </>
          )}
        </div>
      </div>
      <div style={{ marginTop: 12, paddingTop: 10, borderTop: '1px solid rgba(255,255,255,0.07)', fontSize: 11.5, color: '#a1a1a6' }}>
        {t('Or click anywhere in the grid: that adds Hardwave 3xOSC and puts the note where you clicked.')}
      </div>
    </div>
  )
}
