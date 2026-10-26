import { hw } from '../../theme'
import { useRecordingPrefsStore } from '../../stores/recordingPrefsStore'

const buttonStyle = (on = false): React.CSSProperties => ({
  padding: '3px 8px', fontSize: 10, borderRadius: hw.radius.sm, cursor: 'pointer',
  background: on ? hw.accent : 'rgba(255,255,255,0.06)',
  color: on ? '#fff' : hw.textSecondary,
  border: `1px solid ${on ? hw.accent : hw.borderDark}`,
})

/**
 * What a MIDI take does to the notes it is played over: lands on top as a
 * new clip, merges into the clip (blend, Ctrl+B), or replaces them. Before
 * replace existed, playing a part again over the old one left both playing.
 */
export function MidiTakeModeRow() {
  const blend = useRecordingPrefsStore((s) => s.blendRecord)
  const replace = useRecordingPrefsStore((s) => s.replaceRecord)
  const setMode = useRecordingPrefsStore((s) => s.setMidiTakeMode)
  const mode = replace ? 'replace' : blend ? 'blend' : 'stack'
  return (
    <div style={{
      display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 8, marginTop: 8, padding: '8px 12px',
      background: hw.bgPanel, borderRadius: hw.radius.sm, border: `1px solid ${hw.borderDark}`,
    }}>
      <span style={{ fontSize: 11, color: hw.textMuted }}>A MIDI take over existing notes</span>
      <div style={{ display: 'flex', gap: 4 }}>
        <button style={buttonStyle(mode === 'stack')} onClick={() => setMode('stack')} title="The take is a new clip on top; the old notes keep playing">New clip</button>
        <button style={buttonStyle(mode === 'blend')} onClick={() => setMode('blend')} title="The take's notes join the clip under it (blend, Ctrl+B)">Merge</button>
        <button style={buttonStyle(mode === 'replace')} onClick={() => setMode('replace')} title="The notes under the take are removed, so only the take plays there">Replace</button>
      </div>
    </div>
  )
}
