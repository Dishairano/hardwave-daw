// Phase 10 — resource stress extremes. How does the engine handle
// thousands of entities, huge clip counts, giant tempo maps, and whole
// projects with all of the above?
//
// Tests are self-cleaning: every one restores initial state before
// returning (new_project or targeted removals).

import { invoke } from '@tauri-apps/api/core'
import { devDumpState } from './devApi'
import type { TestDef } from './tests'

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

async function engineAlive(): Promise<boolean> {
  try { return Number.isFinite((await devDumpState()).bpm) } catch { return false }
}

export const PHASE10_TESTS: TestDef[] = []

PHASE10_TESTS.push(
  {
    id: 'p10_1000_tempo_entries',
    kind: 'AUTO',
    phase: 10,
    phase1Item: 'Tempo map with 1000 entries',
    title: 'Add 1000 tempo entries, verify all present, remove',
    instructions: 'Creates 1000 entries at stride 100 ticks; expects readback count ≥ 1000; new_project at end.',
    run: async ({ log }) => {
      const N = 1000
      for (let i = 1; i <= N; i++) {
        try { await invoke('add_tempo_entry', { tick: i * 100, bpm: 100 + (i % 100), ramp: 'instant' }) } catch {}
      }
      const entries = await invoke<any[]>('get_tempo_entries')
      const ok = entries.length >= N // initial + added
      log(ok ? 'pass' : 'fail', 'count', { expected: `>=${N}`, actual: entries.length })
      await invoke('new_project')
      return { pass: ok, note: `${entries.length} entries` }
    },
  },
  {
    id: 'p10_500_midi_tracks',
    kind: 'AUTO',
    phase: 10,
    phase1Item: '500 MIDI tracks',
    title: 'Add 500 MIDI tracks and remove them all',
    instructions: 'Stresses track list + graph rebuild; engine must stay alive.',
    run: async ({ log }) => {
      const ids: string[] = []
      for (let i = 0; i < 500; i++) {
        try { ids.push(await invoke<string>('add_midi_track', { name: `M${i}` })) } catch {}
      }
      const mid = (await invoke<any[]>('get_tracks')).length
      const alive1 = await engineAlive()
      for (const id of ids) { try { await invoke('remove_track', { trackId: id }) } catch {} }
      const alive2 = await engineAlive()
      const ok = alive1 && alive2 && mid >= 500
      log(ok ? 'pass' : 'fail', 'stress', { expected: '>=500', actual: mid })
      await invoke('new_project')
      return { pass: ok, note: `peak=${mid}` }
    },
  },
  {
    id: 'p10_1000_midi_notes_one_clip',
    kind: 'AUTO',
    phase: 10,
    phase1Item: '1000 MIDI notes in one clip',
    title: 'Add 1000 notes, verify, delete all',
    instructions: 'Heavy loop — should survive.',
    run: async ({ log }) => {
      const trackId = await invoke<string>('add_midi_track', { name: 'MidiStress' })
      const clipId = await invoke<string>('create_midi_clip', { trackId, lengthTicks: 960000 })
      for (let i = 0; i < 1000; i++) {
        try { await invoke('add_midi_note', { trackId, clipId, pitch: 60 + (i % 24), startTick: i * 240, durationTicks: 120 }) } catch {}
      }
      const notes = await invoke<any[]>('get_midi_notes', { trackId, clipId })
      const ok = notes.length === 1000 && await engineAlive()
      log(ok ? 'pass' : 'fail', 'count', { expected: 1000, actual: notes.length })
      await invoke('new_project')
      return { pass: ok, note: `${notes.length}` }
    },
  },
  {
    id: 'p10_100_clips_one_track',
    kind: 'AUTO',
    phase: 10,
    phase1Item: '100 clips on one track',
    title: 'Import 100 audio clips onto one track',
    instructions: 'Stresses clip array + waveform peak requests.',
    run: async ({ log }) => {
      const trackId = await invoke<string>('add_audio_track', { name: 'ClipStress' })
      const fsPath = await invoke<string>('dev_resolve_test_asset', { name: 'sine-440-1s.wav' })
      for (let i = 0; i < 100; i++) {
        try { await invoke('import_audio_file', { trackId, filePath: fsPath, positionTicks: i * 960 }) } catch {}
      }
      const clips = await invoke<any[]>('get_track_clips', { trackId })
      const ok = clips.length === 100 && await engineAlive()
      log(ok ? 'pass' : 'fail', 'count', { expected: 100, actual: clips.length })
      await invoke('new_project')
      return { pass: ok, note: `${clips.length}` }
    },
  },
  {
    id: 'p10_50_sends_fanout_and_removal',
    kind: 'AUTO',
    phase: 10,
    phase1Item: '50-send fan-out + bulk removal',
    title: '50 sends from one source, then remove from both ends',
    instructions: 'Create sends, delete half by index, verify remainder, then clean up.',
    run: async ({ log }) => {
      const src = await invoke<string>('add_audio_track', { name: 'FanStress' })
      const targets: string[] = []
      for (let i = 0; i < 50; i++) {
        const t = await invoke<string>('add_audio_track', { name: `FT${i}` })
        targets.push(t)
        await invoke('add_send', { trackId: src, targetId: t })
      }
      const full = (await invoke<any[]>('get_sends', { trackId: src })).length
      // Remove every other index (descending to keep indices valid)
      for (let i = full - 1; i >= 0; i -= 2) {
        try { await invoke('remove_send', { trackId: src, sendIndex: i }) } catch {}
      }
      const remaining = (await invoke<any[]>('get_sends', { trackId: src })).length
      const ok = full === 50 && remaining === 25 && await engineAlive()
      log(ok ? 'pass' : 'fail', 'sends', { expected: '50→25', actual: `${full}→${remaining}` })
      await invoke('new_project')
      return { pass: ok, note: `${full}→${remaining}` }
    },
  },
  {
    id: 'p10_huge_project_save_load',
    kind: 'AUTO',
    phase: 10,
    phase1Item: 'Save/load project with 50 tracks + 200 tempo entries + 500 MIDI notes',
    title: 'Huge project round-trips through disk',
    instructions: 'Builds a large project, saves, new, loads, verifies counts survive.',
    run: async ({ log }) => {
      // Build
      const trackIds: string[] = []
      for (let i = 0; i < 50; i++) {
        trackIds.push(await invoke<string>('add_audio_track', { name: `HugeT${i}` }))
      }
      for (let i = 1; i <= 200; i++) {
        try { await invoke('add_tempo_entry', { tick: i * 240, bpm: 100 + (i % 60), ramp: 'instant' }) } catch {}
      }
      const midiId = await invoke<string>('add_midi_track', { name: 'HugeMidi' })
      const clipId = await invoke<string>('create_midi_clip', { trackId: midiId, lengthTicks: 480000 })
      for (let i = 0; i < 500; i++) {
        try { await invoke('add_midi_note', { trackId: midiId, clipId, pitch: 48 + (i % 36), startTick: i * 480, durationTicks: 240 }) } catch {}
      }
      const trackCountBefore = (await invoke<any[]>('get_tracks')).length
      const tempoCountBefore = (await invoke<any[]>('get_tempo_entries')).length
      const notesBefore = (await invoke<any[]>('get_midi_notes', { trackId: midiId, clipId })).length

      const path = `/tmp/daw_huge_${Date.now()}.hwp`
      await invoke('save_project', { path })
      await invoke('new_project')
      await invoke('load_project', { path })

      const tracksAfter = await invoke<any[]>('get_tracks')
      const midiAfter = tracksAfter.find((t) => t.name === 'HugeMidi')
      let notesAfter = 0
      if (midiAfter) {
        const clips = await invoke<any[]>('get_track_clips', { trackId: midiAfter.id })
        if (clips.length > 0) {
          notesAfter = (await invoke<any[]>('get_midi_notes', { trackId: midiAfter.id, clipId: clips[0].id })).length
        }
      }
      const tempoAfter = (await invoke<any[]>('get_tempo_entries')).length
      const ok = tracksAfter.length === trackCountBefore && tempoAfter === tempoCountBefore && notesAfter === notesBefore
      log(ok ? 'pass' : 'fail', 'roundtrip', {
        expected: `${trackCountBefore}/${tempoCountBefore}/${notesBefore}`,
        actual: `${tracksAfter.length}/${tempoAfter}/${notesAfter}`,
      })
      await invoke('new_project')
      return { pass: ok, note: `${tracksAfter.length} tracks, ${tempoAfter} tempo, ${notesAfter} notes` }
    },
  },
  {
    id: 'p10_rapid_new_project_10',
    kind: 'AUTO',
    phase: 10,
    phase1Item: 'Rapid new_project stress',
    title: '10 new_project calls in sequence',
    instructions: 'Each new_project clears state and rebuilds the graph — shouldn\'t leak.',
    run: async ({ log }) => {
      for (let i = 0; i < 10; i++) await invoke('new_project')
      const alive = await engineAlive()
      log(alive ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: alive })
      return { pass: alive, note: 'survived' }
    },
  },
  {
    id: 'p10_rapid_save_load_5_cycles',
    kind: 'AUTO',
    phase: 10,
    phase1Item: 'Save/load soak 5x',
    title: '5 save→new→load cycles on a moderate project',
    instructions: 'Engine stays alive and state survives every cycle.',
    run: async ({ log }) => {
      for (let i = 0; i < 5; i++) {
        const id = await invoke<string>('add_audio_track', { name: `Cycle${i}` })
        const path = `/tmp/daw_cycle_${Date.now()}_${i}.hwp`
        await invoke('save_project', { path })
        await invoke('new_project')
        await invoke('load_project', { path })
        const tracks = await invoke<any[]>('get_tracks')
        if (!tracks.some((t) => t.name === `Cycle${i}`)) {
          log('fail', `cycle ${i}`, { expected: 'present', actual: 'missing' })
          await invoke('new_project')
          return { pass: false, note: `cycle ${i} lost track` }
        }
      }
      const alive = await engineAlive()
      log(alive ? 'pass' : 'fail', '5 cycles', { expected: 'ok', actual: alive })
      await invoke('new_project')
      return { pass: alive, note: '5 cycles survived' }
    },
  },
  {
    id: 'p10_500_undo_cycle',
    kind: 'AUTO',
    phase: 10,
    phase1Item: '500-edit undo stack',
    title: 'Apply 500 edits, undo 500 times',
    instructions: 'Verifies deep undo history does not overflow or crash.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      const orig = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.volume_db
      for (let i = 0; i < 500; i++) await invoke('set_track_volume', { trackId: id, volumeDb: -(i % 50) * 0.1 })
      for (let i = 0; i < 500; i++) { try { await invoke('undo') } catch {} }
      const after = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.volume_db
      const ok = Math.abs(after - orig) < 0.15
      log(ok ? 'pass' : 'fail', 'undo 500', { expected: orig, actual: after })
      return { pass: ok, note: `final=${after}` }
    },
  },
  {
    id: 'p10_large_channel_rack_payload',
    kind: 'AUTO',
    phase: 10,
    phase1Item: 'Channel rack 1 MB payload',
    title: 'set_channel_rack_state with ~1 MB payload roundtrips',
    instructions: 'Large opaque payload should round-trip through get.',
    run: async ({ log }) => {
      const prev = await invoke<string | null>('get_channel_rack_state')
      const big = JSON.stringify({ channels: Array.from({ length: 1000 }, (_, i) => ({ name: `Ch${i}`, pattern: 'x'.repeat(1000) })) })
      await invoke('set_channel_rack_state', { payload: big })
      const after = await invoke<string | null>('get_channel_rack_state')
      const ok = after === big
      log(ok ? 'pass' : 'fail', '1MB roundtrip', { expected: big.length, actual: after?.length })
      await invoke('set_channel_rack_state', { payload: prev })
      return { pass: ok, note: `len=${after?.length}` }
    },
  },
  {
    id: 'p10_rapid_track_reorder',
    kind: 'AUTO',
    phase: 10,
    phase1Item: 'Rapid track reorder',
    title: '100 reorders on a 5-track project',
    instructions: 'Random reorders to stress graph rebuild path.',
    run: async ({ log }) => {
      const ids: string[] = []
      for (let i = 0; i < 5; i++) ids.push(await invoke<string>('add_audio_track', { name: `RO${i}` }))
      for (let i = 0; i < 100; i++) {
        const t = ids[Math.floor(Math.random() * ids.length)]
        const newIdx = Math.floor(Math.random() * 10)
        try { await invoke('reorder_track', { trackId: t, newIndex: newIdx }) } catch {}
      }
      const alive = await engineAlive()
      log(alive ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: alive })
      await invoke('new_project')
      return { pass: alive, note: 'survived' }
    },
  },
)
