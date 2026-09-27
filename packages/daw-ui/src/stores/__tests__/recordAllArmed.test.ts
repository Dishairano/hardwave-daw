import { describe, it, expect, vi, beforeEach } from 'vitest'

const invokeMock = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invokeMock(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const { useTransportStore } = await import('../transportStore')
const { useTrackStore } = await import('../trackStore')
const { useNotificationStore } = await import('../notificationStore')

function armedTrack(id: string, kind: 'Audio' | 'Midi') {
  return {
    id,
    name: id,
    kind,
    color: '#fff',
    volume_db: 0,
    pan: 0,
    muted: false,
    soloed: false,
    armed: true,
    monitor_input: false,
    clips: [],
    inserts: [],
  }
}

const TAKE = {
  path: '/takes/Take 1.wav',
  seconds: 2,
  peak: 0.5,
  truncated: false,
  passes: [{ path: '/takes/Take 1.wav', startSamples: 0, startTicks: 960, seconds: 2, peak: 0.5 }],
}

/** Answers every command the recording path sends, and records the calls. */
function route(take: unknown) {
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'toggle_recording': return Promise.resolve(take)
      case 'stop': return Promise.resolve(take)
      case 'commit_recording_to_midi_clip': return Promise.resolve(['clip-midi'])
      case 'import_audio_file': return Promise.resolve({ clip_id: 'clip-audio' })
      case 'get_tracks': return Promise.resolve(useTrackStore.getState().tracks)
      default: return Promise.resolve(null)
    }
  })
}

function callsTo(cmd: string) {
  return invokeMock.mock.calls.filter(c => c[0] === cmd)
}

describe('a recording lands on every armed track', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    route(TAKE)
    useNotificationStore.setState({ notifications: [] })
    useTransportStore.setState({ recording: false, recordStartSample: null, positionSamples: 96_000 })
    useTrackStore.setState({
      tracks: [armedTrack('vocal', 'Audio'), armedTrack('synth', 'Midi')],
    } as never)
  })

  it('records audio and MIDI in the same pass', async () => {
    await useTransportStore.getState().toggleRecording() // start
    await useTransportStore.getState().toggleRecording() // stop

    expect(callsTo('import_audio_file')).toHaveLength(1)
    expect(callsTo('import_audio_file')[0][1]).toMatchObject({ trackId: 'vocal' })
    expect(callsTo('commit_recording_to_midi_clip')).toHaveLength(1)
    expect(callsTo('commit_recording_to_midi_clip')[0][1]).toMatchObject({ trackId: 'synth' })
  })

  it('gives each armed audio track the take', async () => {
    useTrackStore.setState({
      tracks: [armedTrack('left', 'Audio'), armedTrack('right', 'Audio')],
    } as never)
    await useTransportStore.getState().toggleRecording()
    await useTransportStore.getState().toggleRecording()

    expect(callsTo('import_audio_file').map(c => (c[1] as { trackId: string }).trackId))
      .toEqual(['left', 'right'])
  })

  it('commits a MIDI take when the spacebar stops it, not only the record button', async () => {
    useTrackStore.setState({ tracks: [armedTrack('synth', 'Midi')] } as never)
    await useTransportStore.getState().toggleRecording() // start
    await useTransportStore.getState().stop()

    expect(callsTo('commit_recording_to_midi_clip')).toHaveLength(1)
  })

  it('takes the whole thing back as one undo step', async () => {
    await useTransportStore.getState().toggleRecording()
    await useTransportStore.getState().toggleRecording()

    expect(callsTo('begin_history_group')).toHaveLength(1)
    expect(callsTo('end_history_group')).toHaveLength(1)
  })

  it('says so when nothing was captured', async () => {
    route({ path: null, seconds: 0, peak: 0, truncated: false, passes: [] })
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'commit_recording_to_midi_clip') return Promise.reject('no MIDI in the window')
      if (cmd === 'toggle_recording') {
        return Promise.resolve({ path: null, seconds: 0, peak: 0, truncated: false, passes: [] })
      }
      if (cmd === 'get_tracks') return Promise.resolve(useTrackStore.getState().tracks)
      return Promise.resolve(null)
    })
    await useTransportStore.getState().toggleRecording()
    await useTransportStore.getState().toggleRecording()

    const warnings = useNotificationStore.getState().notifications.filter(n => n.level === 'warning')
    expect(warnings.map(n => n.message)).toContain('Nothing was recorded')
  })
})
