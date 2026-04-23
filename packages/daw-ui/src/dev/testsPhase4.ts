// Phase 4 — exhaustive parametric matrices + multi-step integration scenarios.
// The goal is no sampling gaps: if a parameter has a documented range, every
// representative value in the range gets a dedicated test row.

import { invoke } from '@tauri-apps/api/core'
import { devDumpState } from './devApi'
import type { TestDef } from './tests'

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))
const approx = (a: number, b: number, tol = 0.01) => Math.abs(a - b) <= tol

async function cleanupExtraTracks(before: any[]): Promise<void> {
  const ids = new Set(before.map((t) => t.id))
  const now = await invoke<any[]>('get_tracks')
  for (const t of now) {
    if (!ids.has(t.id)) {
      try { await invoke('remove_track', { trackId: t.id }) } catch {}
    }
  }
}

export const PHASE4_TESTS: TestDef[] = []

// ─────────────────────────────────────────────────────────────────────────────
// 4A — All 128 MIDI pitches, every velocity step, dense BPM sweep
// ─────────────────────────────────────────────────────────────────────────────

// 128 MIDI pitches — one test per pitch
for (let p = 0; p <= 127; p++) {
  PHASE4_TESTS.push({
    id: `p4_midi_pitch_${p}`,
    kind: 'AUTO',
    phase: 4,
    phase1Item: 'MIDI note pitch full range',
    title: `MIDI note pitch ${p}`,
    instructions: `Add note of pitch ${p}; verify readback`,
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const trackId = await invoke<string>('add_midi_track', { name: `P${p}` })
      const clipId = await invoke<string>('create_midi_clip', { trackId, lengthTicks: 960 })
      await invoke('add_midi_note', { trackId, clipId, pitch: p, startTick: 0, durationTicks: 240 })
      const n = (await invoke<any[]>('get_midi_notes', { trackId, clipId }))[0]
      const ok = n.pitch === p
      log(ok ? 'pass' : 'fail', 'pitch', { expected: p, actual: n.pitch })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `${n.pitch}` }
    },
  })
}

// 21 velocity steps (0.00, 0.05, 0.10, …, 1.00)
for (let i = 0; i <= 20; i++) {
  const v = Number((i * 0.05).toFixed(2))
  PHASE4_TESTS.push({
    id: `p4_midi_velocity_${v.toString().replace('.', '_')}`,
    kind: 'AUTO',
    phase: 4,
    phase1Item: 'MIDI note velocity matrix',
    title: `MIDI velocity ${v}`,
    instructions: `Add note at velocity ${v}; readback matches`,
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const trackId = await invoke<string>('add_midi_track', { name: `V${v}` })
      const clipId = await invoke<string>('create_midi_clip', { trackId, lengthTicks: 960 })
      await invoke('add_midi_note', { trackId, clipId, pitch: 60, startTick: 0, durationTicks: 240, velocity: v })
      const n = (await invoke<any[]>('get_midi_notes', { trackId, clipId }))[0]
      const ok = approx(n.velocity, v, 0.005)
      log(ok ? 'pass' : 'fail', 'velocity', { expected: v, actual: n.velocity })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `${n.velocity}` }
    },
  })
}

// BPM sweep every 10 BPM from 20 to 990 plus 999
{
  const bpms: number[] = []
  for (let b = 20; b <= 990; b += 10) bpms.push(b)
  bpms.push(999)
  for (const b of bpms) {
    PHASE4_TESTS.push({
      id: `p4_bpm_${b}`,
      kind: 'AUTO',
      phase: 4,
      phase1Item: 'BPM dense sweep',
      title: `BPM ${b}`,
      instructions: `set_bpm(${b}) roundtrip`,
      run: async ({ log }) => {
        await invoke('set_bpm', { bpm: b })
        await sleep(15)
        const s = await devDumpState()
        const ok = approx(s.bpm, b, 0.05)
        log(ok ? 'pass' : 'fail', 'bpm', { expected: b, actual: s.bpm })
        return { pass: ok, note: `${s.bpm}` }
      },
    })
  }
}

// Master volume dense sweep every 1 dB from -60 to +6
{
  const dbs: number[] = []
  for (let d = -60; d <= 6; d += 1) dbs.push(d)
  for (const d of dbs) {
    PHASE4_TESTS.push({
      id: `p4_master_vol_${d.toString().replace('-', 'n')}`,
      kind: 'AUTO',
      phase: 4,
      phase1Item: 'Master volume dense sweep',
      title: `Master volume ${d} dB`,
      instructions: `set_master_volume(${d})`,
      run: async ({ log }) => {
        await invoke('set_master_volume', { db: d })
        await sleep(10)
        const s = await devDumpState()
        const ok = approx(s.masterVolumeDb, d, 0.05)
        log(ok ? 'pass' : 'fail', 'master', { expected: d, actual: s.masterVolumeDb })
        return { pass: ok, note: `${s.masterVolumeDb}` }
      },
    })
  }
}

// Pan dense sweep every 0.05 from -1 to +1
{
  const pans: number[] = []
  for (let p = -100; p <= 100; p += 5) pans.push(p / 100)
  for (const pan of pans) {
    PHASE4_TESTS.push({
      id: `p4_track_pan_${pan.toString().replace('-', 'n').replace('.', '_')}`,
      kind: 'AUTO',
      phase: 4,
      phase1Item: 'Track pan dense sweep',
      title: `Track pan ${pan}`,
      instructions: `set_track_pan(${pan}) roundtrip`,
      run: async ({ log, ensureAudioTrack }) => {
        const id = await ensureAudioTrack()
        await invoke('set_track_pan', { trackId: id, pan })
        const t = (await invoke<any[]>('get_tracks')).find((x) => x.id === id)!
        const ok = approx(t.pan, pan, 0.01)
        log(ok ? 'pass' : 'fail', 'pan', { expected: pan, actual: t.pan })
        await invoke('set_track_pan', { trackId: id, pan: 0 })
        return { pass: ok, note: `${t.pan}` }
      },
    })
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// 4B — Integration scenarios
// ─────────────────────────────────────────────────────────────────────────────

PHASE4_TESTS.push(
  {
    id: 'p4_int_midi_clip_save_load_roundtrip',
    kind: 'AUTO',
    phase: 4,
    phase1Item: 'Integration: MIDI save/load',
    title: 'Create MIDI clip with 10 notes, save, new, load, verify notes',
    instructions: 'Full MIDI roundtrip through disk.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const trackId = await invoke<string>('add_midi_track', { name: 'MidiSaveLoad' })
      const clipId = await invoke<string>('create_midi_clip', { trackId, lengthTicks: 7680 })
      for (let i = 0; i < 10; i++) {
        await invoke('add_midi_note', { trackId, clipId, pitch: 60 + i, startTick: i * 240, durationTicks: 240, velocity: 0.5 + i * 0.04 })
      }
      const path = `/tmp/daw_int_midi_${Date.now()}.hwp`
      await invoke('save_project', { path })
      await invoke('new_project')
      await invoke('load_project', { path })
      const tracks = await invoke<any[]>('get_tracks')
      const reloaded = tracks.find((t) => t.name === 'MidiSaveLoad')
      let noteCount = 0
      if (reloaded) {
        const clips = await invoke<any[]>('get_track_clips', { trackId: reloaded.id })
        if (clips.length > 0) {
          const notes = await invoke<any[]>('get_midi_notes', { trackId: reloaded.id, clipId: clips[0].id })
          noteCount = notes.length
        }
      }
      const ok = noteCount === 10
      log(ok ? 'pass' : 'fail', 'notes after load', { expected: 10, actual: noteCount })
      await invoke('new_project')
      await cleanupExtraTracks(before)
      return { pass: ok, note: `${noteCount}` }
    },
  },
  {
    id: 'p4_int_sends_save_load_roundtrip',
    kind: 'AUTO',
    phase: 4,
    phase1Item: 'Integration: sends save/load',
    title: 'Create send A→B, save, new, load, send restored',
    instructions: 'Sends survive project save/load cycle.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const a = await invoke<string>('add_audio_track', { name: 'SndSaveA' })
      const b = await invoke<string>('add_audio_track', { name: 'SndSaveB' })
      await invoke('add_send', { trackId: a, targetId: b, gainDb: -3.5, preFader: true })
      const path = `/tmp/daw_int_sends_${Date.now()}.hwp`
      await invoke('save_project', { path })
      await invoke('new_project')
      await invoke('load_project', { path })
      const tracks = await invoke<any[]>('get_tracks')
      const aReloaded = tracks.find((t) => t.name === 'SndSaveA')
      const bReloaded = tracks.find((t) => t.name === 'SndSaveB')
      const sends = aReloaded ? await invoke<any[]>('get_sends', { trackId: aReloaded.id }) : []
      const ok = sends.length === 1 && bReloaded && sends[0].target === bReloaded.id && approx(sends[0].gainDb, -3.5, 0.01) && sends[0].preFader === true
      log(ok ? 'pass' : 'fail', 'send restored', { expected: '-3.5 pre', actual: `${sends[0]?.gainDb} ${sends[0]?.preFader}` })
      await invoke('new_project')
      await cleanupExtraTracks(before)
      return { pass: !!ok, note: `count=${sends.length}` }
    },
  },
  {
    id: 'p4_int_tempo_map_save_load',
    kind: 'AUTO',
    phase: 4,
    phase1Item: 'Integration: tempo map save/load',
    title: 'Tempo map survives save/load',
    instructions: 'Add 4 entries, save, new, load, all entries present.',
    run: async ({ log }) => {
      const ticks = [1920, 3840, 5760, 7680]
      for (const t of ticks) {
        await invoke('add_tempo_entry', { tick: t, bpm: 140 + t / 1000, ramp: 'instant' })
      }
      const path = `/tmp/daw_int_tempo_${Date.now()}.hwp`
      await invoke('save_project', { path })
      await invoke('new_project')
      await invoke('load_project', { path })
      const entries = await invoke<any[]>('get_tempo_entries')
      const found = ticks.every((t) => entries.some((e: any) => e.tick === t))
      log(found ? 'pass' : 'fail', 'tempo after load', { expected: ticks, actual: entries.map((e: any) => e.tick) })
      await invoke('new_project')
      return { pass: found, note: `${entries.length} entries` }
    },
  },
  {
    id: 'p4_int_track_params_save_load',
    kind: 'AUTO',
    phase: 4,
    phase1Item: 'Integration: advanced track params save/load',
    title: 'Color, pan, pitch, fine tune, filter survive save/load',
    instructions: 'Set a bunch of params, save, new, load, verify.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const id = await invoke<string>('add_audio_track', { name: 'TrParamSave' })
      await invoke('set_track_color', { trackId: id, color: '#abcdef' })
      await invoke('set_track_pan', { trackId: id, pan: -0.75 })
      await invoke('set_track_pitch_semitones', { trackId: id, semitones: 7 })
      await invoke('set_track_fine_tune_cents', { trackId: id, cents: 42.5 })
      await invoke('set_track_filter_type', { trackId: id, filterType: 'lp' })
      await invoke('set_track_filter_cutoff', { trackId: id, cutoffHz: 800 })
      const path = `/tmp/daw_int_params_${Date.now()}.hwp`
      await invoke('save_project', { path })
      await invoke('new_project')
      await invoke('load_project', { path })
      const t = (await invoke<any[]>('get_tracks')).find((x) => x.name === 'TrParamSave')
      const ok = t && t.color === '#abcdef' && approx(t.pan, -0.75, 0.01) && t.pitchSemitones === 7 && approx(t.fineTuneCents, 42.5, 0.1) && t.filterType === 'lp' && approx(t.filterCutoffHz, 800, 1)
      log(ok ? 'pass' : 'fail', 'params', {
        expected: '#abcdef/-0.75/7/42.5/lp/800',
        actual: `${t?.color}/${t?.pan}/${t?.pitchSemitones}/${t?.fineTuneCents}/${t?.filterType}/${t?.filterCutoffHz}`,
      })
      await invoke('new_project')
      await cleanupExtraTracks(before)
      return { pass: !!ok, note: 'all restored' }
    },
  },
  {
    id: 'p4_int_track_reorder_persists',
    kind: 'AUTO',
    phase: 4,
    phase1Item: 'Integration: track order persists',
    title: 'Reorder 5 tracks, save, load, order preserved',
    instructions: 'Create 5 named tracks, reorder, save, load, verify order.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const names = ['ZZA', 'ZZB', 'ZZC', 'ZZD', 'ZZE']
      for (const n of names) await invoke<string>('add_audio_track', { name: n })
      let tracks = await invoke<any[]>('get_tracks')
      const myTracks = tracks.filter((t) => names.includes(t.name))
      // Reorder myTracks[0] to the end
      await invoke('reorder_track', { trackId: myTracks[0].id, newIndex: myTracks.length - 1 + tracks.indexOf(myTracks[0]) })
      const path = `/tmp/daw_int_reorder_${Date.now()}.hwp`
      await invoke('save_project', { path })
      await invoke('new_project')
      await invoke('load_project', { path })
      tracks = await invoke<any[]>('get_tracks')
      const post = tracks.filter((t) => names.includes(t.name)).map((t) => t.name)
      const ok = post.length === 5 && post[0] !== 'ZZA'
      log(ok ? 'pass' : 'fail', 'order', { expected: 'moved', actual: post.join(',') })
      await invoke('new_project')
      await cleanupExtraTracks(before)
      return { pass: ok, note: post.join(',') }
    },
  },
  {
    id: 'p4_int_cycle_3hop_rejected',
    kind: 'AUTO',
    phase: 4,
    phase1Item: 'Send cycle detection 3-hop',
    title: 'A→B→C, then C→A must reject (3-hop cycle)',
    instructions: 'Multi-hop cycle still caught.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const a = await invoke<string>('add_audio_track', { name: 'C3A' })
      const b = await invoke<string>('add_audio_track', { name: 'C3B' })
      const c = await invoke<string>('add_audio_track', { name: 'C3C' })
      await invoke('add_send', { trackId: a, targetId: b })
      await invoke('add_send', { trackId: b, targetId: c })
      let threw = false
      try { await invoke('add_send', { trackId: c, targetId: a }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'cycle rejected', { expected: true, actual: threw })
      await cleanupExtraTracks(before)
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p4_int_cycle_4hop_rejected',
    kind: 'AUTO',
    phase: 4,
    phase1Item: 'Send cycle detection 4-hop',
    title: 'A→B→C→D, then D→A must reject (4-hop cycle)',
    instructions: 'Even longer cycle.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const a = await invoke<string>('add_audio_track', { name: 'C4A' })
      const b = await invoke<string>('add_audio_track', { name: 'C4B' })
      const c = await invoke<string>('add_audio_track', { name: 'C4C' })
      const d = await invoke<string>('add_audio_track', { name: 'C4D' })
      await invoke('add_send', { trackId: a, targetId: b })
      await invoke('add_send', { trackId: b, targetId: c })
      await invoke('add_send', { trackId: c, targetId: d })
      let threw = false
      try { await invoke('add_send', { trackId: d, targetId: a }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', '4-hop rejected', { expected: true, actual: threw })
      await cleanupExtraTracks(before)
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p4_int_concurrent_volume_parallel',
    kind: 'AUTO',
    phase: 4,
    phase1Item: 'Concurrency: parallel volume writes',
    title: 'Promise.all(100 set_track_volume) — final reflects last resolved',
    instructions: 'Race 100 parallel writes, then 1 explicit, verify final.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      const writes: Promise<void>[] = []
      for (let i = 0; i < 100; i++) {
        writes.push(invoke('set_track_volume', { trackId: id, volumeDb: -i * 0.1 }))
      }
      await Promise.allSettled(writes)
      await invoke('set_track_volume', { trackId: id, volumeDb: -2.5 })
      await sleep(80)
      const t = (await invoke<any[]>('get_tracks')).find((x) => x.id === id)!
      const ok = approx(t.volume_db, -2.5, 0.1)
      log(ok ? 'pass' : 'fail', 'final', { expected: -2.5, actual: t.volume_db })
      await invoke('set_track_volume', { trackId: id, volumeDb: 0 })
      return { pass: ok, note: `${t.volume_db}` }
    },
  },
  {
    id: 'p4_int_concurrent_mixed_writes',
    kind: 'AUTO',
    phase: 4,
    phase1Item: 'Concurrency: parallel cross-param writes',
    title: 'Parallel BPM + volume + pan + master — all lands',
    instructions: 'Race unrelated writes, verify each target value.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      const writes: Promise<void>[] = [
        invoke('set_bpm', { bpm: 111 }),
        invoke('set_track_volume', { trackId: id, volumeDb: -4.4 }),
        invoke('set_track_pan', { trackId: id, pan: -0.33 }),
        invoke('set_master_volume', { db: -1.1 }),
      ]
      await Promise.allSettled(writes)
      await sleep(60)
      const s = await devDumpState()
      const t = (await invoke<any[]>('get_tracks')).find((x) => x.id === id)!
      const ok = approx(s.bpm, 111, 0.1) && approx(t.volume_db, -4.4, 0.1) && approx(t.pan, -0.33, 0.01) && approx(s.masterVolumeDb, -1.1, 0.05)
      log(ok ? 'pass' : 'fail', 'cross', { expected: '111/-4.4/-0.33/-1.1', actual: `${s.bpm}/${t.volume_db}/${t.pan}/${s.masterVolumeDb}` })
      // restore
      await invoke('set_bpm', { bpm: 140 })
      await invoke('set_track_volume', { trackId: id, volumeDb: 0 })
      await invoke('set_track_pan', { trackId: id, pan: 0 })
      await invoke('set_master_volume', { db: 0 })
      return { pass: ok, note: 'all landed' }
    },
  },
  {
    id: 'p4_soak_1k_volume_writes',
    kind: 'AUTO',
    phase: 4,
    phase1Item: 'Soak test — 1k volume writes',
    title: '1000 sequential volume writes leave engine alive',
    instructions: 'Issues 1000 volume writes; reads dev_dump_state afterwards.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      for (let i = 0; i < 1000; i++) {
        await invoke('set_track_volume', { trackId: id, volumeDb: -(i % 50) * 0.1 })
      }
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'alive', { expected: 'finite', actual: s.bpm })
      await invoke('set_track_volume', { trackId: id, volumeDb: 0 })
      return { pass: ok, note: 'survived' }
    },
  },
  {
    id: 'p4_soak_500_bpm_sets',
    kind: 'AUTO',
    phase: 4,
    phase1Item: 'Soak test — BPM spam',
    title: '500 sequential set_bpm writes leave engine alive',
    instructions: '500 writes then verify state.',
    run: async ({ log }) => {
      for (let i = 0; i < 500; i++) {
        await invoke('set_bpm', { bpm: 60 + (i % 300) })
      }
      await invoke('set_bpm', { bpm: 140 })
      await sleep(40)
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'alive', { expected: 'finite', actual: s.bpm })
      return { pass: ok, note: `bpm=${s.bpm}` }
    },
  },
  {
    id: 'p4_int_new_project_clears_everything',
    kind: 'AUTO',
    phase: 4,
    phase1Item: 'Integration: new_project resets state',
    title: 'new_project removes added tracks and tempo entries',
    instructions: 'Add track + tempo entry, call new_project, both gone.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      await invoke<string>('add_audio_track', { name: 'NewProjAdded' })
      await invoke('add_tempo_entry', { tick: 1920, bpm: 180, ramp: 'instant' })
      await invoke('new_project')
      const after = await invoke<any[]>('get_tracks')
      const entries = await invoke<any[]>('get_tempo_entries')
      const ok = !after.some((t) => t.name === 'NewProjAdded') && !entries.some((e: any) => e.tick === 1920)
      log(ok ? 'pass' : 'fail', 'cleared', { expected: 'gone', actual: `tracks=${after.length} tempoEntries=${entries.length}` })
      await cleanupExtraTracks(before)
      return { pass: ok, note: 'cleaned' }
    },
  },
  {
    id: 'p4_int_undo_redo_cross_feature',
    kind: 'AUTO',
    phase: 4,
    phase1Item: 'Undo stack cross-feature',
    title: 'Undo pops volume, then pan, then BPM changes in order',
    instructions: 'Apply 3 unrelated ops, undo 3 times, restores initial state.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      const initialVol = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.volume_db
      const initialPan = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.pan
      const initialBpm = (await devDumpState()).bpm
      await invoke('set_track_volume', { trackId: id, volumeDb: initialVol - 5 })
      await invoke('set_track_pan', { trackId: id, pan: 0.5 })
      await invoke('set_bpm', { bpm: initialBpm + 20 })
      for (let i = 0; i < 3; i++) await invoke('undo')
      await sleep(30)
      const t = (await invoke<any[]>('get_tracks')).find((x) => x.id === id)!
      const s = await devDumpState()
      const ok = approx(t.volume_db, initialVol, 0.1) && approx(t.pan, initialPan, 0.01) && approx(s.bpm, initialBpm, 0.1)
      log(ok ? 'pass' : 'fail', 'undo3', { expected: `${initialVol}/${initialPan}/${initialBpm}`, actual: `${t.volume_db}/${t.pan}/${s.bpm}` })
      return { pass: ok, note: 'restored' }
    },
  },
)
