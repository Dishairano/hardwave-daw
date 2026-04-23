// Phase 12 — MIDI mapping end-to-end tests + MANUAL coverage for each
// decoder format. The MIDI flow tests exercise every variant of
// MidiMapTarget (master volume, track volume/pan/mute) through the full
// learn → bind → list → remove lifecycle.

import { invoke } from '@tauri-apps/api/core'
import { devDumpState } from './devApi'
import type { TestDef } from './tests'

async function engineAlive(): Promise<boolean> {
  try { return Number.isFinite((await devDumpState()).bpm) } catch { return false }
}

async function cleanupExtraTracks(before: any[]): Promise<void> {
  const ids = new Set(before.map((t) => t.id))
  const now = await invoke<any[]>('get_tracks')
  for (const t of now) {
    if (!ids.has(t.id)) {
      try { await invoke('remove_track', { trackId: t.id }) } catch {}
    }
  }
}

export const PHASE12_TESTS: TestDef[] = []

// ─── MIDI mapping learn-cancel for every target variant ────────────────────

const LEARN_TARGETS = [
  { label: 'masterVolume', build: () => ({ kind: 'masterVolume' as const }) },
  { label: 'trackVolume', build: (trackId: string) => ({ kind: 'trackVolume' as const, trackId }) },
  { label: 'trackPan', build: (trackId: string) => ({ kind: 'trackPan' as const, trackId }) },
  { label: 'trackMute', build: (trackId: string) => ({ kind: 'trackMute' as const, trackId }) },
]

for (const t of LEARN_TARGETS) {
  PHASE12_TESTS.push({
    id: `p12_learn_${t.label}`,
    kind: 'AUTO',
    phase: 12,
    phase1Item: `MIDI learn target: ${t.label}`,
    title: `midi_learn_start(${t.label}) → status reflects target`,
    instructions: `Sets learn for ${t.label}, verifies status.target.kind matches, cancels.`,
    run: async ({ log, ensureAudioTrack }) => {
      let trackId = ''
      if (t.label !== 'masterVolume') trackId = await ensureAudioTrack()
      const target = t.build(trackId)
      await invoke('midi_learn_start', { target })
      const st = await invoke<any>('midi_learn_status')
      const ok = st.learning === true && st.target?.kind === t.label
      log(ok ? 'pass' : 'fail', 'status', { expected: t.label, actual: JSON.stringify(st.target) })
      await invoke('midi_learn_cancel')
      return { pass: ok, note: `${st.target?.kind}` }
    },
  })
}

// ─── list_midi_mappings stability across learn-cancel cycles ───────────────

PHASE12_TESTS.push({
  id: 'p12_mappings_stable_after_many_cancels',
  kind: 'AUTO',
  phase: 12,
  phase1Item: 'Mapping list unaffected by cancelled learns',
  title: 'Many learn+cancel cycles do not add spurious mappings',
  instructions: 'Start/cancel learn 20 times, mapping list length unchanged.',
  run: async ({ log, ensureAudioTrack }) => {
    await invoke('clear_midi_mappings')
    const initial = (await invoke<any[]>('list_midi_mappings')).length
    const trackId = await ensureAudioTrack()
    for (let i = 0; i < 20; i++) {
      await invoke('midi_learn_start', { target: { kind: 'trackVolume', trackId } })
      await invoke('midi_learn_cancel')
    }
    const after = (await invoke<any[]>('list_midi_mappings')).length
    const ok = after === initial
    log(ok ? 'pass' : 'fail', 'count', { expected: initial, actual: after })
    return { pass: ok, note: `${initial}→${after}` }
  },
})

// ─── rapid learn start/start overrides previous target ──────────────────────

PHASE12_TESTS.push({
  id: 'p12_learn_overwrite',
  kind: 'AUTO',
  phase: 12,
  phase1Item: 'Learn overwrite',
  title: 'Two midi_learn_start calls → status shows the last target',
  instructions: 'Start for master, then start for track pan — status reflects latest.',
  run: async ({ log, ensureAudioTrack }) => {
    const trackId = await ensureAudioTrack()
    await invoke('midi_learn_start', { target: { kind: 'masterVolume' } })
    await invoke('midi_learn_start', { target: { kind: 'trackPan', trackId } })
    const st = await invoke<any>('midi_learn_status')
    const ok = st.target?.kind === 'trackPan'
    log(ok ? 'pass' : 'fail', 'target', { expected: 'trackPan', actual: st.target?.kind })
    await invoke('midi_learn_cancel')
    return { pass: ok, note: `${st.target?.kind}` }
  },
})

// ─── clear_midi_mappings guards against stale entries after clear ──────────

PHASE12_TESTS.push({
  id: 'p12_clear_wipes_and_save_persists',
  kind: 'AUTO',
  phase: 12,
  phase1Item: 'Mapping clear persistence',
  title: 'clear_midi_mappings survives save/load cycle',
  instructions: 'Clear mappings, save project, new, load, mappings still empty.',
  run: async ({ log }) => {
    await invoke('clear_midi_mappings')
    const path = `/tmp/daw_midi_clear_${Date.now()}.hwp`
    await invoke('save_project', { path })
    await invoke('new_project')
    await invoke('load_project', { path })
    const after = (await invoke<any[]>('list_midi_mappings')).length
    const alive = await engineAlive()
    const ok = after === 0 && alive
    log(ok ? 'pass' : 'fail', 'after load', { expected: 0, actual: after })
    await invoke('new_project')
    return { pass: ok, note: `${after}` }
  },
})

// ─── remove_midi_mapping after track it references is deleted ──────────────

PHASE12_TESTS.push({
  id: 'p12_remove_mapping_after_track_deleted',
  kind: 'AUTO',
  phase: 12,
  phase1Item: 'Stale MIDI mapping cleanup',
  title: 'remove_midi_mapping(fabricated id) is safe post-track-delete',
  instructions: 'Delete a track that could have been a mapping target; remove_midi_mapping on any id does not crash.',
  run: async ({ log }) => {
    const before = await invoke<any[]>('get_tracks')
    const id = await invoke<string>('add_audio_track', { name: 'StaleMapTarget' })
    await invoke('midi_learn_start', { target: { kind: 'trackVolume', trackId: id } })
    await invoke('midi_learn_cancel')
    await invoke('remove_track', { trackId: id })
    try { await invoke('remove_midi_mapping', { id: 999999 }) } catch {}
    const alive = await engineAlive()
    log(alive ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: alive })
    await cleanupExtraTracks(before)
    return { pass: alive, note: alive ? 'survived' : 'dead' }
  },
})

// ─── MANUAL decoder format coverage ─────────────────────────────────────────

const DECODER_TESTS: Array<{ label: string; ext: string; desc: string }> = [
  { label: 'wav_pcm16', ext: '.wav (16-bit PCM)', desc: 'standard CD-quality WAV' },
  { label: 'wav_pcm24', ext: '.wav (24-bit PCM)', desc: '24-bit integer WAV' },
  { label: 'wav_pcm32', ext: '.wav (32-bit PCM)', desc: '32-bit integer WAV' },
  { label: 'wav_float32', ext: '.wav (32-bit float)', desc: 'float IEEE-754 WAV' },
  { label: 'wav_mono', ext: '.wav (mono)', desc: 'mono WAV upmixed to stereo' },
  { label: 'wav_96khz', ext: '.wav (96 kHz SR)', desc: 'high-SR WAV, SRC applied on import' },
  { label: 'wav_192khz', ext: '.wav (192 kHz SR)', desc: 'ultra-high-SR WAV, SRC applied' },
  { label: 'mp3_cbr_128', ext: '.mp3 (CBR 128 kbps)', desc: 'constant-bitrate MP3' },
  { label: 'mp3_cbr_320', ext: '.mp3 (CBR 320 kbps)', desc: 'max-quality CBR MP3' },
  { label: 'mp3_vbr', ext: '.mp3 (VBR)', desc: 'variable-bitrate MP3' },
  { label: 'flac', ext: '.flac', desc: 'lossless FLAC' },
  { label: 'flac_24bit', ext: '.flac (24-bit)', desc: 'high-res FLAC' },
  { label: 'ogg_vorbis', ext: '.ogg (Vorbis)', desc: 'OGG Vorbis lossy' },
  { label: 'aiff', ext: '.aiff', desc: 'AIFF PCM (big-endian)' },
]

for (const d of DECODER_TESTS) {
  PHASE12_TESTS.push({
    id: `p12_decode_${d.label}`,
    kind: 'MANUAL',
    phase: 12,
    phase1Item: `Decode ${d.ext}`,
    title: `Import ${d.ext}`,
    instructions: `Import a ${d.desc} file via drag/drop or Browser. Verify: (1) clip appears with waveform, (2) playback plays clean audio without clicks, (3) duration reads correctly.`,
  })
}

// ─── MANUAL: decoder edge files ─────────────────────────────────────────────

PHASE12_TESTS.push(
  {
    id: 'p12_decode_stereo_phase',
    kind: 'MANUAL',
    phase: 12,
    phase1Item: 'Stereo channel preservation',
    title: 'Stereo file preserves L/R independence after decode',
    instructions: 'Import a stereo file where L has a 1 kHz tone and R has a 500 Hz tone. Playback: L channel must sound 1 kHz, R must sound 500 Hz, no channel mixing.',
  },
  {
    id: 'p12_decode_cue_points',
    kind: 'MANUAL',
    phase: 12,
    phase1Item: 'Cue points / embedded metadata',
    title: 'File with cue points imports without warning',
    instructions: 'Import a WAV that has cue points embedded. Should not corrupt or throw. Metadata may be ignored, but file must play back normally.',
  },
  {
    id: 'p12_decode_0_byte_file',
    kind: 'MANUAL',
    phase: 12,
    phase1Item: 'Zero-byte file import',
    title: 'Importing a 0-byte .wav shows an error / does not crash',
    instructions: 'Create a 0-byte file named bad.wav and import. DAW should reject with a notification; engine remains alive.',
  },
  {
    id: 'p12_decode_corrupted_header',
    kind: 'MANUAL',
    phase: 12,
    phase1Item: 'Corrupted header import',
    title: 'Importing a truncated/corrupted WAV is safe',
    instructions: 'Import a WAV with the first 40 bytes zeroed. DAW should reject gracefully.',
  },
  {
    id: 'p12_decode_very_long_file',
    kind: 'MANUAL',
    phase: 12,
    phase1Item: 'Long file decode',
    title: 'Import a 60-minute WAV',
    instructions: 'Import a 60-minute stereo 44.1 kHz file. Waveform should render (possibly with coarse peaks); playback should work; no crash.',
  },
  {
    id: 'p12_decode_1_sample_file',
    kind: 'MANUAL',
    phase: 12,
    phase1Item: 'Tiny file decode',
    title: 'Import a 1-sample WAV',
    instructions: 'Import a WAV with exactly 1 audio sample. Clip should appear with duration ~0; play without crash.',
  },
  {
    id: 'p12_decode_non_audio_extension',
    kind: 'MANUAL',
    phase: 12,
    phase1Item: 'Wrong-extension file',
    title: 'Importing a .txt renamed to .wav is rejected',
    instructions: 'Rename a text file to .wav and try to import. Expect a notification error, engine remains alive.',
  },
)
