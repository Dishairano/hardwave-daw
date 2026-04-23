// Phase 11 — entity-lifecycle races.
//
// What happens when the entity a command refers to disappears mid-flight?
// What if two operations race on the same thing? This phase forces those
// corner cases and verifies the engine stays alive and its state remains
// consistent.

import { invoke } from '@tauri-apps/api/core'
import { devDumpState } from './devApi'
import type { TestDef } from './tests'

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

async function cleanupExtraTracks(before: any[]): Promise<void> {
  const ids = new Set(before.map((t) => t.id))
  const now = await invoke<any[]>('get_tracks')
  for (const t of now) {
    if (!ids.has(t.id)) {
      try { await invoke('remove_track', { trackId: t.id }) } catch {}
    }
  }
}

async function engineAlive(): Promise<boolean> {
  try { return Number.isFinite((await devDumpState()).bpm) } catch { return false }
}

export const PHASE11_TESTS: TestDef[] = []

PHASE11_TESTS.push(
  {
    id: 'p11_delete_send_target_track',
    kind: 'AUTO',
    phase: 11,
    phase1Item: 'Delete send target',
    title: 'Removing a track that is a send target does not orphan audio',
    instructions: 'Create A→B, remove B, verify A still exists and engine alive.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const a = await invoke<string>('add_audio_track', { name: 'LR_A' })
      const b = await invoke<string>('add_audio_track', { name: 'LR_B' })
      await invoke('add_send', { trackId: a, targetId: b })
      await invoke('remove_track', { trackId: b })
      const tracks = await invoke<any[]>('get_tracks')
      const aStillThere = tracks.some((t) => t.id === a)
      const bGone = !tracks.some((t) => t.id === b)
      const alive = await engineAlive()
      const ok = aStillThere && bGone && alive
      log(ok ? 'pass' : 'fail', 'state', { expected: 'A alive, B gone', actual: `a=${aStillThere} b=${bGone} alive=${alive}` })
      await cleanupExtraTracks(before)
      return { pass: ok, note: ok ? 'graceful' : 'broken' }
    },
  },
  {
    id: 'p11_delete_output_bus_target',
    kind: 'AUTO',
    phase: 11,
    phase1Item: 'Delete output bus target',
    title: 'Removing a track that is another track\'s outputBus',
    instructions: 'Route A→B output, remove B, A must still be listed.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const a = await invoke<string>('add_audio_track', { name: 'OB_A' })
      const b = await invoke<string>('add_audio_track', { name: 'OB_B' })
      await invoke('set_track_output_bus', { trackId: a, outputBus: b })
      await invoke('remove_track', { trackId: b })
      const tracks = await invoke<any[]>('get_tracks')
      const alive = await engineAlive()
      const aStillThere = tracks.some((t) => t.id === a)
      const ok = aStillThere && alive
      log(ok ? 'pass' : 'fail', 'alive', { expected: 'A alive, engine ok', actual: `a=${aStillThere} alive=${alive}` })
      await cleanupExtraTracks(before)
      return { pass: ok, note: ok ? 'graceful' : 'broken' }
    },
  },
  {
    id: 'p11_delete_sidechain_source',
    kind: 'AUTO',
    phase: 11,
    phase1Item: 'Delete sidechain source',
    title: 'Removing a track used as sidechain source does not crash',
    instructions: 'Set src as sidechain for a bogus slot (expected-reject), then remove src.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const src = await invoke<string>('add_audio_track', { name: 'SC_Src' })
      const dst = await invoke<string>('add_audio_track', { name: 'SC_Dst' })
      // Bogus slot id — source validation still triggers the liveness path.
      try { await invoke('set_plugin_sidechain_source', { trackId: dst, slotId: '__nope__', sourceTrackId: src }) } catch {}
      await invoke('remove_track', { trackId: src })
      const alive = await engineAlive()
      log(alive ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: alive })
      await cleanupExtraTracks(before)
      return { pass: alive, note: alive ? 'survived' : 'dead' }
    },
  },
  {
    id: 'p11_remove_track_then_edit_clip',
    kind: 'AUTO',
    phase: 11,
    phase1Item: 'Clip edit after track removal',
    title: 'Editing a clip on a removed track rejects but engine lives',
    instructions: 'Import clip, cache ids, remove track, then attempt set_clip_gain.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const trackId = await invoke<string>('add_audio_track', { name: 'DelClipTrack' })
      const fsPath = await invoke<string>('dev_resolve_test_asset', { name: 'sine-440-1s.wav' })
      await invoke('import_audio_file', { trackId, filePath: fsPath, positionTicks: 0 })
      const [clip] = await invoke<any[]>('get_track_clips', { trackId })
      await invoke('remove_track', { trackId })
      let threw = false
      try { await invoke('set_clip_gain', { trackId, clipId: clip.id, gainDb: -6 }) } catch { threw = true }
      const alive = await engineAlive()
      const ok = threw && alive
      log(ok ? 'pass' : 'fail', 'rejected+alive', { expected: 'threw+alive', actual: `threw=${threw} alive=${alive}` })
      await cleanupExtraTracks(before)
      return { pass: ok, note: ok ? 'graceful' : 'broken' }
    },
  },
  {
    id: 'p11_parallel_delete_and_edit',
    kind: 'AUTO',
    phase: 11,
    phase1Item: 'Parallel track delete + volume edit',
    title: 'remove_track and set_track_volume fired in parallel',
    instructions: 'Races removal against an edit; engine must stay alive regardless of order.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const id = await invoke<string>('add_audio_track', { name: 'RaceVol' })
      const ops = [
        invoke('remove_track', { trackId: id }),
        invoke('set_track_volume', { trackId: id, volumeDb: -12 }),
      ]
      await Promise.allSettled(ops)
      const alive = await engineAlive()
      log(alive ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: alive })
      await cleanupExtraTracks(before)
      return { pass: alive, note: alive ? 'survived' : 'dead' }
    },
  },
  {
    id: 'p11_parallel_clip_delete_and_move',
    kind: 'AUTO',
    phase: 11,
    phase1Item: 'Parallel clip delete + move',
    title: 'delete_clip and move_clip fired in parallel',
    instructions: 'Races clip operations on the same id.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const trackId = await invoke<string>('add_audio_track', { name: 'RaceClip' })
      const fsPath = await invoke<string>('dev_resolve_test_asset', { name: 'sine-440-1s.wav' })
      await invoke('import_audio_file', { trackId, filePath: fsPath, positionTicks: 0 })
      const [clip] = await invoke<any[]>('get_track_clips', { trackId })
      await Promise.allSettled([
        invoke('delete_clip', { trackId, clipId: clip.id }),
        invoke('move_clip', { trackId, clipId: clip.id, newPositionTicks: 1920 }),
      ])
      const alive = await engineAlive()
      log(alive ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: alive })
      await cleanupExtraTracks(before)
      return { pass: alive, note: alive ? 'survived' : 'dead' }
    },
  },
  {
    id: 'p11_parallel_new_project_and_edit',
    kind: 'AUTO',
    phase: 11,
    phase1Item: 'Parallel new_project + edit',
    title: 'new_project racing against set_bpm keeps engine alive',
    instructions: 'Fire both, let them race, read back state.',
    run: async ({ log }) => {
      await Promise.allSettled([
        invoke('new_project'),
        invoke('set_bpm', { bpm: 175 }),
        invoke('set_master_volume', { db: -3 }),
      ])
      await sleep(40)
      const alive = await engineAlive()
      log(alive ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: alive })
      await invoke('new_project')
      return { pass: alive, note: alive ? 'survived' : 'dead' }
    },
  },
  {
    id: 'p11_parallel_load_and_transport',
    kind: 'AUTO',
    phase: 11,
    phase1Item: 'Load project during playback',
    title: 'load_project fired with play active',
    instructions: 'Start play, fire load_project of a saved file, verify engine alive and playing=false post-load.',
    run: async ({ log }) => {
      const path = `/tmp/daw_race_load_${Date.now()}.hwp`
      await invoke('save_project', { path })
      await invoke('play')
      await sleep(40)
      try { await invoke('load_project', { path }) } catch {}
      await sleep(40)
      await invoke('stop')
      const alive = await engineAlive()
      log(alive ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: alive })
      return { pass: alive, note: alive ? 'survived' : 'dead' }
    },
  },
  {
    id: 'p11_remove_tempo_entry_during_playback',
    kind: 'AUTO',
    phase: 11,
    phase1Item: 'Tempo map edit during playback',
    title: 'Add + remove tempo entry while playing keeps engine alive',
    instructions: 'Start play, add a tempo entry, remove it, stop.',
    run: async ({ log }) => {
      await invoke('play')
      await sleep(40)
      await invoke('add_tempo_entry', { tick: 1920, bpm: 200, ramp: 'instant' })
      let entries = await invoke<any[]>('get_tempo_entries')
      const idx = entries.findIndex((e: any) => e.tick === 1920)
      if (idx > 0) await invoke('remove_tempo_entry', { index: idx })
      await invoke('stop')
      const alive = await engineAlive()
      log(alive ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: alive })
      return { pass: alive, note: alive ? 'survived' : 'dead' }
    },
  },
  {
    id: 'p11_edit_clip_during_playback',
    kind: 'AUTO',
    phase: 11,
    phase1Item: 'Clip edit during playback',
    title: 'set_clip_gain during active playback is graceful',
    instructions: 'Play, set gain, stop, verify alive.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const trackId = await invoke<string>('add_audio_track', { name: 'PlayEdit' })
      const fsPath = await invoke<string>('dev_resolve_test_asset', { name: 'sine-440-1s.wav' })
      await invoke('import_audio_file', { trackId, filePath: fsPath, positionTicks: 0 })
      const [clip] = await invoke<any[]>('get_track_clips', { trackId })
      await invoke('play')
      await sleep(50)
      await invoke('set_clip_gain', { trackId, clipId: clip.id, gainDb: -12 })
      await invoke('stop')
      const alive = await engineAlive()
      log(alive ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: alive })
      await cleanupExtraTracks(before)
      return { pass: alive, note: alive ? 'survived' : 'dead' }
    },
  },
  {
    id: 'p11_send_on_now_removed_track_target',
    kind: 'AUTO',
    phase: 11,
    phase1Item: 'Mutate send after target removed',
    title: 'set_send_gain after target was removed',
    instructions: 'A→B then remove B then try set_send_gain on that send — must not crash.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const a = await invoke<string>('add_audio_track', { name: 'STM_A' })
      const b = await invoke<string>('add_audio_track', { name: 'STM_B' })
      const idx = await invoke<number>('add_send', { trackId: a, targetId: b })
      await invoke('remove_track', { trackId: b })
      try { await invoke('set_send_gain', { trackId: a, sendIndex: idx, gainDb: -3 }) } catch {}
      const alive = await engineAlive()
      log(alive ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: alive })
      await cleanupExtraTracks(before)
      return { pass: alive, note: alive ? 'survived' : 'dead' }
    },
  },
  {
    id: 'p11_midi_note_after_clip_deleted',
    kind: 'AUTO',
    phase: 11,
    phase1Item: 'MIDI note after clip removed',
    title: 'add_midi_note on a deleted MIDI clip rejects',
    instructions: 'Create clip, delete clip, try add_midi_note on same ids.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const trackId = await invoke<string>('add_midi_track', { name: 'MidiDel' })
      const clipId = await invoke<string>('create_midi_clip', { trackId, lengthTicks: 1920 })
      await invoke('delete_clip', { trackId, clipId })
      let threw = false
      try { await invoke('add_midi_note', { trackId, clipId, pitch: 60, startTick: 0, durationTicks: 240 }) } catch { threw = true }
      const alive = await engineAlive()
      const ok = threw && alive
      log(ok ? 'pass' : 'fail', 'rejected', { expected: 'threw+alive', actual: `threw=${threw} alive=${alive}` })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `${threw}/${alive}` }
    },
  },
  {
    id: 'p11_rapid_audio_host_hotswap',
    kind: 'AUTO',
    phase: 11,
    phase1Item: 'Audio host hot-swap stress',
    title: 'Cycle audio host 10× and return to original',
    instructions: 'Toggle between every available host repeatedly.',
    run: async ({ log }) => {
      const original = await invoke<string>('get_audio_host')
      const hosts = await invoke<string[]>('list_audio_hosts')
      if (hosts.length === 0) {
        log('pass', 'no hosts', { expected: 'skip', actual: 'ok' })
        return { pass: true, note: 'skipped — no hosts' }
      }
      for (let i = 0; i < 10; i++) {
        const h = hosts[i % hosts.length]
        try { await invoke('set_audio_host', { hostName: h }) } catch {}
      }
      try { await invoke('set_audio_host', { hostName: original }) } catch {}
      const alive = await engineAlive()
      log(alive ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: alive })
      return { pass: alive, note: alive ? 'survived' : 'dead' }
    },
  },
)
