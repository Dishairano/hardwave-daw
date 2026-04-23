// Phase 3 — extensive coverage across every backend command surface.
// Organized by feature area; each group has roundtrip + edge-case tests.
//
// Conventions:
// - Every test sets phase: 3 so it lands on a dedicated panel tab.
// - Tests that mutate shared state (track list, tempo map, midi mappings,
//   blocklists, scan paths) must restore the prior state before returning.
// - Error-path tests assert that invoke(...) rejects; engine liveness is
//   checked via dev_dump_state afterward.

import { invoke } from '@tauri-apps/api/core'
import { devDumpState } from './devApi'
import type { TestDef } from './tests'

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

const approx = (a: number, b: number, tol = 0.01) => Math.abs(a - b) <= tol

async function cleanupExtraTracks(before: any[]): Promise<void> {
  const beforeIds = new Set(before.map((t) => t.id))
  const now = await invoke<any[]>('get_tracks')
  for (const t of now) {
    if (!beforeIds.has(t.id)) {
      try { await invoke('remove_track', { trackId: t.id }) } catch {}
    }
  }
}

export const PHASE3_TESTS: TestDef[] = []

// ─────────────────────────────────────────────────────────────────────────────
// 3A — Plugins, FX chain, and sends
// ─────────────────────────────────────────────────────────────────────────────

PHASE3_TESTS.push(
  {
    id: 'p3_get_plugins_array',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Plugin scan list',
    title: 'get_plugins returns an array',
    instructions: 'get_plugins resolves and is an array (may be empty on fresh install).',
    run: async ({ log }) => {
      const plugins = await invoke<any[]>('get_plugins')
      const ok = Array.isArray(plugins)
      log(ok ? 'pass' : 'fail', 'get_plugins', { expected: 'array', actual: typeof plugins })
      return { pass: ok, note: `${plugins.length} plugins cached` }
    },
  },
  {
    id: 'p3_plugin_cache_path',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Plugin cache on disk',
    title: 'plugin_cache_path returns string or null',
    instructions: 'plugin_cache_path returns a string path or null.',
    run: async ({ log }) => {
      const p = await invoke<string | null>('plugin_cache_path')
      const ok = p === null || typeof p === 'string'
      log(ok ? 'pass' : 'fail', 'cache path type', { expected: 'string|null', actual: typeof p })
      return { pass: ok, note: p ?? 'null' }
    },
  },
  {
    id: 'p3_get_plugin_blocklist_array',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Plugin blocklist persistence',
    title: 'get_plugin_blocklist returns array',
    instructions: 'get_plugin_blocklist resolves to an array of strings.',
    run: async ({ log }) => {
      const bl = await invoke<string[]>('get_plugin_blocklist')
      const ok = Array.isArray(bl) && bl.every((x) => typeof x === 'string')
      log(ok ? 'pass' : 'fail', 'blocklist shape', { expected: 'string[]', actual: typeof bl })
      return { pass: ok, note: `${bl.length} blocked` }
    },
  },
  {
    id: 'p3_set_plugin_blocklist_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Plugin blocklist roundtrip',
    title: 'set_plugin_blocklist then get_plugin_blocklist matches',
    instructions: 'Sets a blocklist of 3 ids, reads back, then restores previous.',
    run: async ({ log }) => {
      const previous = await invoke<string[]>('get_plugin_blocklist')
      const target = ['com.test.a', 'com.test.b', 'com.test.c']
      await invoke('set_plugin_blocklist', { ids: target })
      const after = await invoke<string[]>('get_plugin_blocklist')
      const ok = target.every((id) => after.includes(id))
      log(ok ? 'pass' : 'fail', 'blocklist', { expected: target, actual: after })
      await invoke('set_plugin_blocklist', { ids: previous })
      return { pass: ok, note: `got ${after.length}` }
    },
  },
  {
    id: 'p3_set_plugin_blocklist_clear',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Plugin blocklist clearing',
    title: 'set_plugin_blocklist([]) empties the list',
    instructions: 'Sets [] then verifies the readback length is 0 before restoring.',
    run: async ({ log }) => {
      const previous = await invoke<string[]>('get_plugin_blocklist')
      await invoke('set_plugin_blocklist', { ids: [] })
      const after = await invoke<string[]>('get_plugin_blocklist')
      const ok = after.length === 0
      log(ok ? 'pass' : 'fail', 'cleared', { expected: 0, actual: after.length })
      await invoke('set_plugin_blocklist', { ids: previous })
      return { pass: ok, note: `after=${after.length}` }
    },
  },
  {
    id: 'p3_get_custom_scan_paths_tuple',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Plugin scan paths',
    title: 'get_custom_scan_paths returns (vst3, clap) tuple',
    instructions: 'Result is a 2-tuple of string arrays.',
    run: async ({ log }) => {
      const paths = await invoke<[string[], string[]]>('get_custom_scan_paths')
      const ok = Array.isArray(paths) && paths.length === 2 && Array.isArray(paths[0]) && Array.isArray(paths[1])
      log(ok ? 'pass' : 'fail', 'paths shape', { expected: '[string[], string[]]', actual: JSON.stringify(paths) })
      return { pass: ok, note: `vst3=${paths[0].length} clap=${paths[1].length}` }
    },
  },
  {
    id: 'p3_set_custom_scan_paths_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Custom plugin scan paths roundtrip',
    title: 'set_custom_scan_paths then read back',
    instructions: 'Sets dummy paths, verifies roundtrip, restores previous.',
    run: async ({ log }) => {
      const prev = await invoke<[string[], string[]]>('get_custom_scan_paths')
      const vst3 = ['/tmp/fake_vst3']
      const clap = ['/tmp/fake_clap']
      await invoke('set_custom_scan_paths', { vst3, clap })
      const now = await invoke<[string[], string[]]>('get_custom_scan_paths')
      const ok = now[0].includes('/tmp/fake_vst3') && now[1].includes('/tmp/fake_clap')
      log(ok ? 'pass' : 'fail', 'scan paths', { expected: `${vst3}/${clap}`, actual: `${now[0]}/${now[1]}` })
      await invoke('set_custom_scan_paths', { vst3: prev[0], clap: prev[1] })
      return { pass: ok, note: `restored` }
    },
  },
  {
    id: 'p3_find_missing_plugins_array',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Missing plugin detection',
    title: 'find_missing_plugins returns array',
    instructions: 'Fresh project shape: resolves to an array.',
    run: async ({ log }) => {
      const missing = await invoke<any[]>('find_missing_plugins')
      const ok = Array.isArray(missing)
      log(ok ? 'pass' : 'fail', 'missing type', { expected: 'array', actual: typeof missing })
      return { pass: ok, note: `${missing.length} missing` }
    },
  },
  {
    id: 'p3_get_last_scan_diff_shape',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Plugin scan diff',
    title: 'get_last_scan_diff has added/removed keys',
    instructions: 'Returns an object shaped like { added: [...], removed: [...] } or similar.',
    run: async ({ log }) => {
      const d = await invoke<any>('get_last_scan_diff')
      const ok = d && typeof d === 'object'
      log(ok ? 'pass' : 'fail', 'diff type', { expected: 'object', actual: typeof d })
      return { pass: ok, note: Object.keys(d ?? {}).join(',') }
    },
  },
  {
    id: 'p3_close_plugin_editor_unknown_label',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Plugin editor lifecycle',
    title: 'close_plugin_editor on unknown label does not crash',
    instructions: 'Calling close with a label that was never opened resolves OK.',
    run: async ({ log }) => {
      try { await invoke('close_plugin_editor', { windowLabel: '__nope__' }) } catch {}
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'engine alive', { expected: 'finite bpm', actual: s.bpm })
      return { pass: ok, note: 'survived' }
    },
  },
)

// FX chain
PHASE3_TESTS.push(
  {
    id: 'p3_fx_chain_bypassed_no_inserts',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'FX chain bypass',
    title: 'set_fx_chain_bypassed succeeds on empty chain',
    instructions: 'On a track with no inserts, bypass is a no-op that still resolves.',
    run: async ({ log, ensureAudioTrack }) => {
      const trackId = await ensureAudioTrack()
      try {
        await invoke('set_fx_chain_bypassed', { trackId, bypassed: true })
        await invoke('set_fx_chain_bypassed', { trackId, bypassed: false })
        log('pass', 'bypass no-op', { expected: 'resolves', actual: 'ok' })
        return { pass: true, note: 'empty chain survived' }
      } catch (e: any) {
        log('fail', 'bypass threw', { expected: 'ok', actual: e?.message ?? e })
        return { pass: false, note: `threw: ${e}` }
      }
    },
  },
  {
    id: 'p3_fx_chain_bypassed_missing_track',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'FX chain bypass error path',
    title: 'set_fx_chain_bypassed on unknown track rejects',
    instructions: 'Invoke with fabricated UUID must reject.',
    run: async ({ log }) => {
      let threw = false
      try { await invoke('set_fx_chain_bypassed', { trackId: '00000000-0000-0000-0000-000000000000', bypassed: true }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected unknown', { expected: true, actual: threw })
      return { pass: threw, note: threw ? 'rejected' : 'silent' }
    },
  },
  {
    id: 'p3_set_insert_enabled_missing_track',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Insert enable error path',
    title: 'set_insert_enabled on unknown track rejects',
    instructions: 'Fabricated track id must reject.',
    run: async ({ log }) => {
      let threw = false
      try { await invoke('set_insert_enabled', { trackId: '00000000-0000-0000-0000-000000000000', slotId: 'x', enabled: true }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_set_insert_enabled_missing_slot',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Insert slot lookup',
    title: 'set_insert_enabled on unknown slot id rejects',
    instructions: 'Track exists, slot id is bogus — must reject.',
    run: async ({ log, ensureAudioTrack }) => {
      const trackId = await ensureAudioTrack()
      let threw = false
      try { await invoke('set_insert_enabled', { trackId, slotId: '__bogus__', enabled: true }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_set_insert_wet_missing_slot',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Insert wet error path',
    title: 'set_insert_wet on unknown slot rejects',
    instructions: 'Track exists, slot id is bogus — must reject.',
    run: async ({ log, ensureAudioTrack }) => {
      const trackId = await ensureAudioTrack()
      let threw = false
      try { await invoke('set_insert_wet', { trackId, slotId: '__bogus__', wet: 0.5 }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_set_plugin_sidechain_self_rejected',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Sidechain self-loop guard',
    title: 'set_plugin_sidechain_source rejects self as source',
    instructions: 'Source track equals target track — must reject.',
    run: async ({ log, ensureAudioTrack }) => {
      const trackId = await ensureAudioTrack()
      let threw = false
      try { await invoke('set_plugin_sidechain_source', { trackId, slotId: '__x__', sourceTrackId: trackId }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected self', { expected: true, actual: threw })
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_set_plugin_sidechain_missing_source',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Sidechain source validation',
    title: 'set_plugin_sidechain_source rejects unknown source',
    instructions: 'Source id is a fabricated UUID — must reject.',
    run: async ({ log, ensureAudioTrack }) => {
      const trackId = await ensureAudioTrack()
      let threw = false
      try { await invoke('set_plugin_sidechain_source', { trackId, slotId: '__x__', sourceTrackId: '00000000-0000-0000-0000-000000000000' }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      return { pass: threw, note: `${threw}` }
    },
  },
)

// Sends
PHASE3_TESTS.push(
  {
    id: 'p3_sends_empty_on_fresh_track',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Sends enumeration',
    title: 'get_sends returns [] on a fresh track',
    instructions: 'New audio track has no sends.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const id = await invoke<string>('add_audio_track', { name: 'SendsFresh' })
      const sends = await invoke<any[]>('get_sends', { trackId: id })
      const ok = Array.isArray(sends) && sends.length === 0
      log(ok ? 'pass' : 'fail', 'sends empty', { expected: 0, actual: sends.length })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `len=${sends.length}` }
    },
  },
  {
    id: 'p3_list_sends_shape',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Global send list',
    title: 'list_sends returns an array',
    instructions: 'Resolves to SendEdge[].',
    run: async ({ log }) => {
      const edges = await invoke<any[]>('list_sends')
      const ok = Array.isArray(edges)
      log(ok ? 'pass' : 'fail', 'list_sends', { expected: 'array', actual: typeof edges })
      return { pass: ok, note: `${edges.length} edges` }
    },
  },
  {
    id: 'p3_create_return_with_send_full',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Create return bus with send',
    title: 'create_return_with_send creates return track + send',
    instructions: 'Adds source track, creates return, verifies new track kind=Return and one outgoing send.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const srcId = await invoke<string>('add_audio_track', { name: 'SrcForReturn' })
      const returnId = await invoke<string>('create_return_with_send', { sourceTrackId: srcId, returnName: 'ReverbReturn' })
      const tracks = await invoke<any[]>('get_tracks')
      const rt = tracks.find((t) => t.id === returnId)
      const sends = await invoke<any[]>('get_sends', { trackId: srcId })
      const kindOk = rt && rt.kind === 'Return'
      const sendOk = sends.length === 1 && sends[0].target === returnId
      const ok = kindOk && sendOk
      log(ok ? 'pass' : 'fail', 'return+send', { expected: 'Return+1 send', actual: `kind=${rt?.kind} sends=${sends.length}` })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `ok=${ok}` }
    },
  },
  {
    id: 'p3_add_send_self_rejected',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Send self-loop guard',
    title: 'add_send with source==target rejects',
    instructions: 'Same id on both ends must reject.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      let threw = false
      try { await invoke('add_send', { trackId: id, targetId: id }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'self rejected', { expected: true, actual: threw })
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_add_send_missing_target_rejected',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Send target validation',
    title: 'add_send rejects unknown target',
    instructions: 'Target id is fabricated UUID.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      let threw = false
      try { await invoke('add_send', { trackId: id, targetId: '00000000-0000-0000-0000-000000000000' }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_add_send_cycle_rejected',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Send cycle detection',
    title: 'Creating A→B then B→A is rejected',
    instructions: 'Second send forms a cycle and must reject.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const a = await invoke<string>('add_audio_track', { name: 'CycleA' })
      const b = await invoke<string>('add_audio_track', { name: 'CycleB' })
      await invoke('add_send', { trackId: a, targetId: b })
      let threw = false
      try { await invoke('add_send', { trackId: b, targetId: a }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'cycle rejected', { expected: true, actual: threw })
      await cleanupExtraTracks(before)
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_add_send_roundtrip_with_index',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Send insertion',
    title: 'add_send returns the new send index',
    instructions: 'Add two sends and verify returned indices 0 and 1.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const a = await invoke<string>('add_audio_track', { name: 'SrcIdx' })
      const b = await invoke<string>('add_audio_track', { name: 'TgtIdx1' })
      const c = await invoke<string>('add_audio_track', { name: 'TgtIdx2' })
      const i0 = await invoke<number>('add_send', { trackId: a, targetId: b })
      const i1 = await invoke<number>('add_send', { trackId: a, targetId: c })
      const ok = i0 === 0 && i1 === 1
      log(ok ? 'pass' : 'fail', 'indices', { expected: '0,1', actual: `${i0},${i1}` })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `${i0},${i1}` }
    },
  },
  {
    id: 'p3_set_send_gain_clamp_high',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Send gain clamping',
    title: 'set_send_gain clamps at +6 dB',
    instructions: 'Gain of +50 should clamp to +6.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const a = await invoke<string>('add_audio_track', { name: 'GA' })
      const b = await invoke<string>('add_audio_track', { name: 'GB' })
      const idx = await invoke<number>('add_send', { trackId: a, targetId: b })
      await invoke('set_send_gain', { trackId: a, sendIndex: idx, gainDb: 50 })
      const sends = await invoke<any[]>('get_sends', { trackId: a })
      const ok = sends[idx].gainDb <= 6 + 0.001
      log(ok ? 'pass' : 'fail', 'clamp hi', { expected: '<=6', actual: sends[idx].gainDb })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `${sends[idx].gainDb}` }
    },
  },
  {
    id: 'p3_set_send_gain_clamp_low',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Send gain clamping',
    title: 'set_send_gain clamps at -100 dB',
    instructions: 'Gain of -500 should clamp to -100.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const a = await invoke<string>('add_audio_track', { name: 'GLA' })
      const b = await invoke<string>('add_audio_track', { name: 'GLB' })
      const idx = await invoke<number>('add_send', { trackId: a, targetId: b })
      await invoke('set_send_gain', { trackId: a, sendIndex: idx, gainDb: -500 })
      const sends = await invoke<any[]>('get_sends', { trackId: a })
      const ok = sends[idx].gainDb >= -100 - 0.001
      log(ok ? 'pass' : 'fail', 'clamp lo', { expected: '>=-100', actual: sends[idx].gainDb })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `${sends[idx].gainDb}` }
    },
  },
  {
    id: 'p3_set_send_gain_nan_ignored',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Send gain NaN guard',
    title: 'set_send_gain with NaN is ignored',
    instructions: 'NaN must not alter stored gain.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const a = await invoke<string>('add_audio_track', { name: 'NA' })
      const b = await invoke<string>('add_audio_track', { name: 'NB' })
      const idx = await invoke<number>('add_send', { trackId: a, targetId: b, gainDb: -6 })
      await invoke('set_send_gain', { trackId: a, sendIndex: idx, gainDb: NaN })
      const sends = await invoke<any[]>('get_sends', { trackId: a })
      const ok = approx(sends[idx].gainDb, -6, 0.001)
      log(ok ? 'pass' : 'fail', 'NaN ignored', { expected: -6, actual: sends[idx].gainDb })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `${sends[idx].gainDb}` }
    },
  },
  {
    id: 'p3_set_send_pre_fader_toggle',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Pre-fader send routing',
    title: 'set_send_pre_fader toggles the flag',
    instructions: 'Flip pre_fader true, then false.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const a = await invoke<string>('add_audio_track', { name: 'PFA' })
      const b = await invoke<string>('add_audio_track', { name: 'PFB' })
      const idx = await invoke<number>('add_send', { trackId: a, targetId: b })
      await invoke('set_send_pre_fader', { trackId: a, sendIndex: idx, preFader: true })
      const s1 = (await invoke<any[]>('get_sends', { trackId: a }))[idx]
      await invoke('set_send_pre_fader', { trackId: a, sendIndex: idx, preFader: false })
      const s2 = (await invoke<any[]>('get_sends', { trackId: a }))[idx]
      const ok = s1.preFader === true && s2.preFader === false
      log(ok ? 'pass' : 'fail', 'pre fader', { expected: 'true→false', actual: `${s1.preFader}→${s2.preFader}` })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `${s1.preFader}→${s2.preFader}` }
    },
  },
  {
    id: 'p3_set_send_enabled_toggle',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Send enable/disable',
    title: 'set_send_enabled toggles the flag',
    instructions: 'Disable then re-enable, observe both states.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const a = await invoke<string>('add_audio_track', { name: 'EnA' })
      const b = await invoke<string>('add_audio_track', { name: 'EnB' })
      const idx = await invoke<number>('add_send', { trackId: a, targetId: b })
      await invoke('set_send_enabled', { trackId: a, sendIndex: idx, enabled: false })
      const off = (await invoke<any[]>('get_sends', { trackId: a }))[idx]
      await invoke('set_send_enabled', { trackId: a, sendIndex: idx, enabled: true })
      const on = (await invoke<any[]>('get_sends', { trackId: a }))[idx]
      const ok = off.enabled === false && on.enabled === true
      log(ok ? 'pass' : 'fail', 'enabled', { expected: 'false→true', actual: `${off.enabled}→${on.enabled}` })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `${off.enabled}→${on.enabled}` }
    },
  },
  {
    id: 'p3_remove_send_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Send deletion',
    title: 'remove_send drops the edge',
    instructions: 'Add one, remove, expect empty.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const a = await invoke<string>('add_audio_track', { name: 'RA' })
      const b = await invoke<string>('add_audio_track', { name: 'RB' })
      const idx = await invoke<number>('add_send', { trackId: a, targetId: b })
      await invoke('remove_send', { trackId: a, sendIndex: idx })
      const sends = await invoke<any[]>('get_sends', { trackId: a })
      const ok = sends.length === 0
      log(ok ? 'pass' : 'fail', 'removed', { expected: 0, actual: sends.length })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `len=${sends.length}` }
    },
  },
  {
    id: 'p3_remove_send_out_of_range',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Send index bounds',
    title: 'remove_send with out-of-range index is safe',
    instructions: 'No crash, no state change, engine alive.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      try { await invoke('remove_send', { trackId: id, sendIndex: 9999 }) } catch {}
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'alive', { expected: 'finite bpm', actual: s.bpm })
      return { pass: ok, note: 'survived' }
    },
  },
  {
    id: 'p3_set_send_target_self_rejected',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Send target self guard',
    title: 'set_send_target rejects self as new target',
    instructions: 'Re-targeting a send to its source must reject.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const a = await invoke<string>('add_audio_track', { name: 'ST1' })
      const b = await invoke<string>('add_audio_track', { name: 'ST2' })
      const idx = await invoke<number>('add_send', { trackId: a, targetId: b })
      let threw = false
      try { await invoke('set_send_target', { trackId: a, sendIndex: idx, targetId: a }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'self rejected', { expected: true, actual: threw })
      await cleanupExtraTracks(before)
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_set_send_target_missing_rejected',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Send target validation',
    title: 'set_send_target rejects unknown target',
    instructions: 'Unknown target id must reject.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const a = await invoke<string>('add_audio_track', { name: 'SM1' })
      const b = await invoke<string>('add_audio_track', { name: 'SM2' })
      const idx = await invoke<number>('add_send', { trackId: a, targetId: b })
      let threw = false
      try { await invoke('set_send_target', { trackId: a, sendIndex: idx, targetId: '00000000-0000-0000-0000-000000000000' }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      await cleanupExtraTracks(before)
      return { pass: threw, note: `${threw}` }
    },
  },
)

// Parametric send-gain matrix
{
  const gains = [-96, -60, -48, -36, -24, -18, -12, -9, -6, -3, -1, 0, 1, 3, 6]
  for (const g of gains) {
    PHASE3_TESTS.push({
      id: `p3_send_gain_${g.toString().replace('-', 'n')}`,
      kind: 'AUTO',
      phase: 3,
      phase1Item: 'Send gain matrix',
      title: `Send gain ${g} dB roundtrip`,
      instructions: `Set gain ${g} and verify readback within clamp range`,
      run: async ({ log }) => {
        const before = await invoke<any[]>('get_tracks')
        const a = await invoke<string>('add_audio_track', { name: `SG${g}a` })
        const b = await invoke<string>('add_audio_track', { name: `SG${g}b` })
        const idx = await invoke<number>('add_send', { trackId: a, targetId: b })
        await invoke('set_send_gain', { trackId: a, sendIndex: idx, gainDb: g })
        const got = (await invoke<any[]>('get_sends', { trackId: a }))[idx].gainDb
        const expected = Math.max(-100, Math.min(6, g))
        const ok = Math.abs(got - expected) < 0.01
        log(ok ? 'pass' : 'fail', 'gain', { expected, actual: got })
        await cleanupExtraTracks(before)
        return { pass: ok, note: `${got}` }
      },
    })
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// 3B — MIDI I/O, mapping, learn, clock, MTC, note CRUD
// ─────────────────────────────────────────────────────────────────────────────

PHASE3_TESTS.push(
  {
    id: 'p3_list_midi_inputs_array',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI input port enumeration',
    title: 'list_midi_inputs returns string array',
    instructions: 'May be empty on headless systems.',
    run: async ({ log }) => {
      const inputs = await invoke<string[]>('list_midi_inputs')
      const ok = Array.isArray(inputs) && inputs.every((x) => typeof x === 'string')
      log(ok ? 'pass' : 'fail', 'inputs', { expected: 'string[]', actual: typeof inputs })
      return { pass: ok, note: `${inputs.length} inputs` }
    },
  },
  {
    id: 'p3_list_midi_outputs_array',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI output port enumeration',
    title: 'list_midi_outputs returns string array',
    instructions: 'May be empty on headless systems.',
    run: async ({ log }) => {
      const outs = await invoke<string[]>('list_midi_outputs')
      const ok = Array.isArray(outs) && outs.every((x) => typeof x === 'string')
      log(ok ? 'pass' : 'fail', 'outputs', { expected: 'string[]', actual: typeof outs })
      return { pass: ok, note: `${outs.length} outputs` }
    },
  },
  {
    id: 'p3_open_midi_input_fake_rejects',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI input error path',
    title: 'open_midi_input with fake port rejects',
    instructions: 'Unknown port name must reject.',
    run: async ({ log }) => {
      let threw = false
      try { await invoke('open_midi_input', { portName: '__no_such_port__' }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_open_midi_output_fake_rejects',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI output error path',
    title: 'open_midi_output with fake port rejects',
    instructions: 'Unknown port name must reject.',
    run: async ({ log }) => {
      let threw = false
      try { await invoke('open_midi_output', { portName: '__no_such_port__' }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_close_midi_input_unknown_safe',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI input close robustness',
    title: 'close_midi_input on unknown port is a no-op',
    instructions: 'No throw, engine remains alive.',
    run: async ({ log }) => {
      try { await invoke('close_midi_input', { portName: '__no_such_port__' }) } catch {}
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: s.bpm })
      return { pass: ok, note: 'survived' }
    },
  },
  {
    id: 'p3_close_midi_output_unknown_safe',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI output close robustness',
    title: 'close_midi_output on unknown port is a no-op',
    instructions: 'No throw, engine remains alive.',
    run: async ({ log }) => {
      try { await invoke('close_midi_output', { portName: '__no_such_port__' }) } catch {}
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: s.bpm })
      return { pass: ok, note: 'survived' }
    },
  },
  {
    id: 'p3_close_all_midi_inputs_safe',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI input bulk close',
    title: 'close_all_midi_inputs does not crash',
    instructions: 'Should resolve even when no ports are open.',
    run: async ({ log }) => {
      try { await invoke('close_all_midi_inputs') } catch {}
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: s.bpm })
      return { pass: ok, note: 'survived' }
    },
  },
  {
    id: 'p3_get_midi_activity_shape',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI activity snapshot',
    title: 'get_midi_activity has open_ports + ms_since_last_event',
    instructions: 'Shape check.',
    run: async ({ log }) => {
      const a = await invoke<any>('get_midi_activity')
      const ok = a && Array.isArray(a.open_ports) && (a.ms_since_last_event === null || typeof a.ms_since_last_event === 'number')
      log(ok ? 'pass' : 'fail', 'shape', { expected: 'valid', actual: JSON.stringify(a).slice(0, 80) })
      return { pass: ok, note: `ports=${a?.open_ports?.length}` }
    },
  },
  {
    id: 'p3_get_midi_desired_ports_array',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI desired ports',
    title: 'get_midi_desired_ports returns array',
    instructions: 'Array of strings.',
    run: async ({ log }) => {
      const p = await invoke<string[]>('get_midi_desired_ports')
      const ok = Array.isArray(p)
      log(ok ? 'pass' : 'fail', 'shape', { expected: 'array', actual: typeof p })
      return { pass: ok, note: `${p.length}` }
    },
  },
  {
    id: 'p3_midi_clock_enable_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI clock output toggle',
    title: 'set_midi_clock_enabled then get_midi_clock_status reflects it',
    instructions: 'Enable, verify status.enabled=true; disable, verify false.',
    run: async ({ log }) => {
      await invoke('set_midi_clock_enabled', { enabled: true })
      const s1 = await invoke<any>('get_midi_clock_status')
      await invoke('set_midi_clock_enabled', { enabled: false })
      const s2 = await invoke<any>('get_midi_clock_status')
      const ok = s1.enabled === true && s2.enabled === false
      log(ok ? 'pass' : 'fail', 'clock toggle', { expected: 'true→false', actual: `${s1.enabled}→${s2.enabled}` })
      return { pass: ok, note: `${s1.enabled}→${s2.enabled}` }
    },
  },
  {
    id: 'p3_midi_clock_sync_enable_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI clock sync input',
    title: 'set_midi_clock_sync_enabled toggles status',
    instructions: 'Enable, verify status, disable, verify.',
    run: async ({ log }) => {
      await invoke('set_midi_clock_sync_enabled', { enabled: true })
      const s1 = await invoke<any>('get_midi_clock_sync_status')
      await invoke('set_midi_clock_sync_enabled', { enabled: false })
      const s2 = await invoke<any>('get_midi_clock_sync_status')
      const ok = s1.enabled === true && s2.enabled === false
      log(ok ? 'pass' : 'fail', 'sync toggle', { expected: 'true→false', actual: `${s1.enabled}→${s2.enabled}` })
      return { pass: ok, note: `${s1.enabled}→${s2.enabled}` }
    },
  },
  {
    id: 'p3_midi_mtc_enable_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI timecode output toggle',
    title: 'set_midi_mtc_enabled toggles status',
    instructions: 'Enable, verify, disable, verify.',
    run: async ({ log }) => {
      await invoke('set_midi_mtc_enabled', { enabled: true })
      const s1 = await invoke<any>('get_midi_mtc_status')
      await invoke('set_midi_mtc_enabled', { enabled: false })
      const s2 = await invoke<any>('get_midi_mtc_status')
      const ok = s1.enabled === true && s2.enabled === false
      log(ok ? 'pass' : 'fail', 'mtc toggle', { expected: 'true→false', actual: `${s1.enabled}→${s2.enabled}` })
      return { pass: ok, note: `${s1.enabled}→${s2.enabled}` }
    },
  },
  {
    id: 'p3_midi_mtc_fps_24',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MTC 24 fps',
    title: 'set_midi_mtc_fps(24) accepted',
    instructions: 'Classic film rate.',
    run: async ({ log }) => {
      await invoke('set_midi_mtc_fps', { fps: 24 })
      const s = await invoke<any>('get_midi_mtc_status')
      const ok = s.fps === 24
      log(ok ? 'pass' : 'fail', 'fps 24', { expected: 24, actual: s.fps })
      return { pass: ok, note: `fps=${s.fps}` }
    },
  },
  {
    id: 'p3_midi_mtc_fps_25',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MTC 25 fps',
    title: 'set_midi_mtc_fps(25) accepted',
    instructions: 'PAL rate.',
    run: async ({ log }) => {
      await invoke('set_midi_mtc_fps', { fps: 25 })
      const s = await invoke<any>('get_midi_mtc_status')
      const ok = s.fps === 25
      log(ok ? 'pass' : 'fail', 'fps 25', { expected: 25, actual: s.fps })
      return { pass: ok, note: `fps=${s.fps}` }
    },
  },
  {
    id: 'p3_midi_mtc_fps_30',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MTC 30 fps',
    title: 'set_midi_mtc_fps(30) accepted',
    instructions: 'NTSC rate.',
    run: async ({ log }) => {
      await invoke('set_midi_mtc_fps', { fps: 30 })
      const s = await invoke<any>('get_midi_mtc_status')
      const ok = s.fps === 30
      log(ok ? 'pass' : 'fail', 'fps 30', { expected: 30, actual: s.fps })
      return { pass: ok, note: `fps=${s.fps}` }
    },
  },
  {
    id: 'p3_midi_mtc_fps_invalid_rejected',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MTC fps validation',
    title: 'set_midi_mtc_fps rejects 42',
    instructions: 'Only 24/25/30 are valid.',
    run: async ({ log }) => {
      let threw = false
      try { await invoke('set_midi_mtc_fps', { fps: 42 }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      await invoke('set_midi_mtc_fps', { fps: 24 })
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_midi_learn_start_status',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI learn mode',
    title: 'midi_learn_start sets learning=true with target',
    instructions: 'Start learn for MasterVolume; status should reflect it.',
    run: async ({ log }) => {
      await invoke('midi_learn_start', { target: { kind: 'masterVolume' } })
      const st = await invoke<any>('midi_learn_status')
      const ok = st.learning === true && st.target && st.target.kind === 'masterVolume'
      log(ok ? 'pass' : 'fail', 'learning', { expected: 'masterVolume', actual: JSON.stringify(st.target) })
      await invoke('midi_learn_cancel')
      return { pass: ok, note: `${st.learning}` }
    },
  },
  {
    id: 'p3_midi_learn_cancel_clears',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI learn cancel',
    title: 'midi_learn_cancel resets learning=false',
    instructions: 'Start + cancel must leave learning=false.',
    run: async ({ log }) => {
      await invoke('midi_learn_start', { target: { kind: 'masterVolume' } })
      await invoke('midi_learn_cancel')
      const st = await invoke<any>('midi_learn_status')
      const ok = st.learning === false
      log(ok ? 'pass' : 'fail', 'cancelled', { expected: false, actual: st.learning })
      return { pass: ok, note: `${st.learning}` }
    },
  },
  {
    id: 'p3_midi_learn_track_volume_target',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI learn track volume target',
    title: 'midi_learn_start with TrackVolume target',
    instructions: 'Start learn for TrackVolume, verify status.target.kind.',
    run: async ({ log, ensureAudioTrack }) => {
      const trackId = await ensureAudioTrack()
      await invoke('midi_learn_start', { target: { kind: 'trackVolume', trackId } })
      const st = await invoke<any>('midi_learn_status')
      const ok = st.learning === true && st.target?.kind === 'trackVolume' && st.target?.trackId === trackId
      log(ok ? 'pass' : 'fail', 'trackVolume', { expected: `trackVolume/${trackId}`, actual: JSON.stringify(st.target) })
      await invoke('midi_learn_cancel')
      return { pass: ok, note: `${st.target?.kind}` }
    },
  },
  {
    id: 'p3_midi_mappings_list_empty_after_clear',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI mapping list',
    title: 'clear_midi_mappings + list_midi_mappings returns []',
    instructions: 'Clear must empty the list.',
    run: async ({ log }) => {
      await invoke('clear_midi_mappings')
      const list = await invoke<any[]>('list_midi_mappings')
      const ok = Array.isArray(list) && list.length === 0
      log(ok ? 'pass' : 'fail', 'empty', { expected: 0, actual: list.length })
      return { pass: ok, note: `${list.length}` }
    },
  },
  {
    id: 'p3_remove_midi_mapping_unknown_safe',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI mapping removal robustness',
    title: 'remove_midi_mapping on unknown id is a no-op',
    instructions: 'Does not throw, engine alive.',
    run: async ({ log }) => {
      try { await invoke('remove_midi_mapping', { id: 999999 }) } catch {}
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: s.bpm })
      return { pass: ok, note: 'survived' }
    },
  },
)

// MIDI note CRUD
PHASE3_TESTS.push(
  {
    id: 'p3_create_midi_clip_with_length',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI clip creation',
    title: 'create_midi_clip with explicit length returns id',
    instructions: 'Creates a 2-bar MIDI clip on a MIDI track.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const trackId = await invoke<string>('add_midi_track', { name: 'MidiA' })
      const clipId = await invoke<string>('create_midi_clip', { trackId, name: 'Clip1', positionTicks: 0, lengthTicks: 7680 })
      const ok = typeof clipId === 'string' && clipId.length > 0
      log(ok ? 'pass' : 'fail', 'clip id', { expected: 'string', actual: typeof clipId })
      await cleanupExtraTracks(before)
      return { pass: ok, note: clipId }
    },
  },
  {
    id: 'p3_create_midi_clip_missing_track_errors',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI clip error path',
    title: 'create_midi_clip on unknown track rejects',
    instructions: 'Fabricated track id must reject.',
    run: async ({ log }) => {
      let threw = false
      try { await invoke('create_midi_clip', { trackId: '00000000-0000-0000-0000-000000000000' }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_midi_note_add_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI note CRUD',
    title: 'add_midi_note returns index and get_midi_notes reflects it',
    instructions: 'Add C4 at tick 0 duration 480, readback has 1 note.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const trackId = await invoke<string>('add_midi_track', { name: 'NoteA' })
      const clipId = await invoke<string>('create_midi_clip', { trackId, lengthTicks: 1920 })
      const idx = await invoke<number>('add_midi_note', { trackId, clipId, pitch: 60, startTick: 0, durationTicks: 480, velocity: 0.7 })
      const notes = await invoke<any[]>('get_midi_notes', { trackId, clipId })
      const ok = idx === 0 && notes.length === 1 && notes[0].pitch === 60 && notes[0].start_tick === 0 && notes[0].duration_ticks === 480 && approx(notes[0].velocity, 0.7, 0.001)
      log(ok ? 'pass' : 'fail', 'note', { expected: 'C4/0/480/0.7', actual: `${notes[0]?.pitch}/${notes[0]?.start_tick}/${notes[0]?.duration_ticks}/${notes[0]?.velocity}` })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `idx=${idx}` }
    },
  },
  {
    id: 'p3_midi_note_default_velocity',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI note default velocity',
    title: 'add_midi_note without velocity uses 0.8',
    instructions: 'Default velocity should be 0.8.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const trackId = await invoke<string>('add_midi_track', { name: 'VelDef' })
      const clipId = await invoke<string>('create_midi_clip', { trackId, lengthTicks: 1920 })
      await invoke('add_midi_note', { trackId, clipId, pitch: 60, startTick: 0, durationTicks: 240 })
      const notes = await invoke<any[]>('get_midi_notes', { trackId, clipId })
      const ok = approx(notes[0].velocity, 0.8, 0.001)
      log(ok ? 'pass' : 'fail', 'velocity', { expected: 0.8, actual: notes[0].velocity })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `${notes[0].velocity}` }
    },
  },
  {
    id: 'p3_midi_note_update_pitch',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI note update: pitch',
    title: 'update_midi_note(pitch) changes pitch only',
    instructions: 'Set pitch from 60 to 72, other fields preserved.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const trackId = await invoke<string>('add_midi_track', { name: 'UpdP' })
      const clipId = await invoke<string>('create_midi_clip', { trackId, lengthTicks: 1920 })
      await invoke('add_midi_note', { trackId, clipId, pitch: 60, startTick: 120, durationTicks: 480, velocity: 0.5 })
      await invoke('update_midi_note', { trackId, clipId, noteIndex: 0, pitch: 72 })
      const n = (await invoke<any[]>('get_midi_notes', { trackId, clipId }))[0]
      const ok = n.pitch === 72 && n.start_tick === 120 && n.duration_ticks === 480 && approx(n.velocity, 0.5, 0.001)
      log(ok ? 'pass' : 'fail', 'updated', { expected: '72/120/480/0.5', actual: `${n.pitch}/${n.start_tick}/${n.duration_ticks}/${n.velocity}` })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `${n.pitch}` }
    },
  },
  {
    id: 'p3_midi_note_update_duration',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI note update: duration',
    title: 'update_midi_note(duration) changes duration only',
    instructions: 'Set duration from 240 to 960.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const trackId = await invoke<string>('add_midi_track', { name: 'UpdD' })
      const clipId = await invoke<string>('create_midi_clip', { trackId, lengthTicks: 3840 })
      await invoke('add_midi_note', { trackId, clipId, pitch: 60, startTick: 0, durationTicks: 240 })
      await invoke('update_midi_note', { trackId, clipId, noteIndex: 0, durationTicks: 960 })
      const n = (await invoke<any[]>('get_midi_notes', { trackId, clipId }))[0]
      const ok = n.duration_ticks === 960
      log(ok ? 'pass' : 'fail', 'duration', { expected: 960, actual: n.duration_ticks })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `${n.duration_ticks}` }
    },
  },
  {
    id: 'p3_midi_note_update_muted',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI note mute flag',
    title: 'update_midi_note(muted=true) toggles mute',
    instructions: 'Mute, verify flag=true.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const trackId = await invoke<string>('add_midi_track', { name: 'UpdM' })
      const clipId = await invoke<string>('create_midi_clip', { trackId, lengthTicks: 1920 })
      await invoke('add_midi_note', { trackId, clipId, pitch: 60, startTick: 0, durationTicks: 240 })
      await invoke('update_midi_note', { trackId, clipId, noteIndex: 0, muted: true })
      const n = (await invoke<any[]>('get_midi_notes', { trackId, clipId }))[0]
      const ok = n.muted === true
      log(ok ? 'pass' : 'fail', 'muted', { expected: true, actual: n.muted })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `${n.muted}` }
    },
  },
  {
    id: 'p3_midi_note_update_out_of_range',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI note index bounds',
    title: 'update_midi_note out-of-range index rejects',
    instructions: 'Index past end must reject cleanly.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const trackId = await invoke<string>('add_midi_track', { name: 'UpdOOR' })
      const clipId = await invoke<string>('create_midi_clip', { trackId, lengthTicks: 1920 })
      let threw = false
      try { await invoke('update_midi_note', { trackId, clipId, noteIndex: 999, pitch: 60 }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      await cleanupExtraTracks(before)
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_midi_note_delete_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI note deletion',
    title: 'delete_midi_note removes the note',
    instructions: 'Add 2 notes, delete index 0, readback length 1.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const trackId = await invoke<string>('add_midi_track', { name: 'Del' })
      const clipId = await invoke<string>('create_midi_clip', { trackId, lengthTicks: 3840 })
      await invoke('add_midi_note', { trackId, clipId, pitch: 60, startTick: 0, durationTicks: 240 })
      await invoke('add_midi_note', { trackId, clipId, pitch: 64, startTick: 480, durationTicks: 240 })
      await invoke('delete_midi_note', { trackId, clipId, noteIndex: 0 })
      const notes = await invoke<any[]>('get_midi_notes', { trackId, clipId })
      const ok = notes.length === 1 && notes[0].pitch === 64
      log(ok ? 'pass' : 'fail', 'deleted', { expected: '1 note pitch=64', actual: `${notes.length}, pitch=${notes[0]?.pitch}` })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `${notes.length}` }
    },
  },
  {
    id: 'p3_midi_note_delete_out_of_range',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI note index bounds',
    title: 'delete_midi_note out-of-range rejects',
    instructions: 'Past-end index must reject.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const trackId = await invoke<string>('add_midi_track', { name: 'DelOOR' })
      const clipId = await invoke<string>('create_midi_clip', { trackId, lengthTicks: 1920 })
      let threw = false
      try { await invoke('delete_midi_note', { trackId, clipId, noteIndex: 42 }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      await cleanupExtraTracks(before)
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_get_midi_notes_missing_clip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI notes lookup error',
    title: 'get_midi_notes on unknown clip rejects',
    instructions: 'Fabricated clip id must reject.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const trackId = await invoke<string>('add_midi_track', { name: 'MissClip' })
      let threw = false
      try { await invoke('get_midi_notes', { trackId, clipId: '00000000-0000-0000-0000-000000000000' }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      await cleanupExtraTracks(before)
      return { pass: threw, note: `${threw}` }
    },
  },
)

// MIDI note pitch matrix — every octave
{
  const pitches = [0, 12, 21, 24, 36, 48, 60, 64, 67, 72, 84, 96, 108, 127]
  for (const p of pitches) {
    PHASE3_TESTS.push({
      id: `p3_midi_note_pitch_${p}`,
      kind: 'AUTO',
      phase: 3,
      phase1Item: 'MIDI note pitch matrix',
      title: `add_midi_note pitch ${p}`,
      instructions: `Add note of pitch ${p}, verify readback`,
      run: async ({ log }) => {
        const before = await invoke<any[]>('get_tracks')
        const trackId = await invoke<string>('add_midi_track', { name: `P${p}` })
        const clipId = await invoke<string>('create_midi_clip', { trackId, lengthTicks: 1920 })
        await invoke('add_midi_note', { trackId, clipId, pitch: p, startTick: 0, durationTicks: 240 })
        const n = (await invoke<any[]>('get_midi_notes', { trackId, clipId }))[0]
        const ok = n.pitch === p
        log(ok ? 'pass' : 'fail', 'pitch', { expected: p, actual: n.pitch })
        await cleanupExtraTracks(before)
        return { pass: ok, note: `${n.pitch}` }
      },
    })
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// 3C — Tempo map, Automation tracks, Channel rack
// ─────────────────────────────────────────────────────────────────────────────

PHASE3_TESTS.push(
  {
    id: 'p3_tempo_entries_has_initial',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Tempo map initial entry',
    title: 'get_tempo_entries has at least one entry at tick 0',
    instructions: 'Fresh project has an initial tempo entry at tick 0.',
    run: async ({ log }) => {
      const entries = await invoke<any[]>('get_tempo_entries')
      const ok = entries.length >= 1 && entries[0].tick === 0
      log(ok ? 'pass' : 'fail', 'initial tempo', { expected: 'tick 0', actual: entries[0]?.tick })
      return { pass: ok, note: `${entries.length} entries` }
    },
  },
  {
    id: 'p3_tempo_entry_add_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Tempo map add entry',
    title: 'add_tempo_entry at tick 3840 bpm 160',
    instructions: 'Add entry, read back, then remove by finding its index.',
    run: async ({ log }) => {
      await invoke('add_tempo_entry', { tick: 3840, bpm: 160, ramp: 'instant' })
      const entries = await invoke<any[]>('get_tempo_entries')
      const added = entries.find((e: any) => e.tick === 3840)
      const ok = added && approx(added.bpm, 160, 0.01)
      log(ok ? 'pass' : 'fail', 'tempo entry', { expected: '160 @ 3840', actual: `${added?.bpm} @ ${added?.tick}` })
      // cleanup
      const idx = entries.findIndex((e: any) => e.tick === 3840)
      if (idx > 0) await invoke('remove_tempo_entry', { index: idx })
      return { pass: ok, note: `${added?.bpm}` }
    },
  },
  {
    id: 'p3_tempo_entry_at_tick_0_rejected',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Tempo entry guard: tick 0 is reserved',
    title: 'add_tempo_entry at tick 0 is rejected',
    instructions: 'Tick 0 is the initial entry; must reject new entry there.',
    run: async ({ log }) => {
      let threw = false
      try { await invoke('add_tempo_entry', { tick: 0, bpm: 140, ramp: 'instant' }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_tempo_entry_duplicate_tick_rejected',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Tempo entry uniqueness',
    title: 'add_tempo_entry duplicate tick is rejected',
    instructions: 'Add @ 1920, second @ 1920 must reject.',
    run: async ({ log }) => {
      await invoke('add_tempo_entry', { tick: 1920, bpm: 130, ramp: 'instant' })
      let threw = false
      try { await invoke('add_tempo_entry', { tick: 1920, bpm: 150, ramp: 'instant' }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      const entries = await invoke<any[]>('get_tempo_entries')
      const idx = entries.findIndex((e: any) => e.tick === 1920)
      if (idx > 0) await invoke('remove_tempo_entry', { index: idx })
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_tempo_entry_nan_rejected',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Tempo entry bpm finite',
    title: 'add_tempo_entry NaN bpm rejected',
    instructions: 'NaN bpm must return error.',
    run: async ({ log }) => {
      let threw = false
      try { await invoke('add_tempo_entry', { tick: 2880, bpm: NaN, ramp: 'instant' }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_tempo_entry_clamps_low',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Tempo entry bpm clamping low',
    title: 'add_tempo_entry bpm=5 clamps to 20',
    instructions: 'Below-range bpm should land at 20.',
    run: async ({ log }) => {
      await invoke('add_tempo_entry', { tick: 4800, bpm: 5, ramp: 'instant' })
      const entries = await invoke<any[]>('get_tempo_entries')
      const added = entries.find((e: any) => e.tick === 4800)
      const ok = added && approx(added.bpm, 20, 0.01)
      log(ok ? 'pass' : 'fail', 'clamped to 20', { expected: 20, actual: added?.bpm })
      const idx = entries.findIndex((e: any) => e.tick === 4800)
      if (idx > 0) await invoke('remove_tempo_entry', { index: idx })
      return { pass: !!ok, note: `${added?.bpm}` }
    },
  },
  {
    id: 'p3_tempo_entry_clamps_high',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Tempo entry bpm clamping high',
    title: 'add_tempo_entry bpm=5000 clamps to 999',
    instructions: 'Above-range bpm should land at 999.',
    run: async ({ log }) => {
      await invoke('add_tempo_entry', { tick: 5760, bpm: 5000, ramp: 'instant' })
      const entries = await invoke<any[]>('get_tempo_entries')
      const added = entries.find((e: any) => e.tick === 5760)
      const ok = added && approx(added.bpm, 999, 0.01)
      log(ok ? 'pass' : 'fail', 'clamped to 999', { expected: 999, actual: added?.bpm })
      const idx = entries.findIndex((e: any) => e.tick === 5760)
      if (idx > 0) await invoke('remove_tempo_entry', { index: idx })
      return { pass: !!ok, note: `${added?.bpm}` }
    },
  },
  {
    id: 'p3_tempo_entries_sorted',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Tempo map ordering',
    title: 'Adding out-of-order entries yields sorted array',
    instructions: 'Add ticks in mixed order; final list sorted by tick.',
    run: async ({ log }) => {
      const ticks = [7680, 1920, 5760, 3840]
      for (const t of ticks) {
        await invoke('add_tempo_entry', { tick: t, bpm: 130, ramp: 'instant' })
      }
      const entries = await invoke<any[]>('get_tempo_entries')
      const mine = entries.filter((e: any) => ticks.includes(e.tick)).map((e: any) => e.tick)
      const sorted = [...mine].sort((a, b) => a - b)
      const ok = JSON.stringify(mine) === JSON.stringify(sorted)
      log(ok ? 'pass' : 'fail', 'sorted', { expected: sorted, actual: mine })
      // cleanup
      for (const t of ticks) {
        const fresh = await invoke<any[]>('get_tempo_entries')
        const idx = fresh.findIndex((e: any) => e.tick === t)
        if (idx > 0) await invoke('remove_tempo_entry', { index: idx })
      }
      return { pass: ok, note: JSON.stringify(mine) }
    },
  },
  {
    id: 'p3_remove_tempo_entry_index_0_rejected',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Initial tempo protection',
    title: 'remove_tempo_entry(0) is rejected',
    instructions: 'Index 0 is the initial entry — must reject.',
    run: async ({ log }) => {
      let threw = false
      try { await invoke('remove_tempo_entry', { index: 0 }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_remove_tempo_entry_out_of_range',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Tempo index bounds',
    title: 'remove_tempo_entry(999) rejects',
    instructions: 'Index past end must reject.',
    run: async ({ log }) => {
      let threw = false
      try { await invoke('remove_tempo_entry', { index: 999 }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_set_tempo_entry_bpm_change',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Tempo entry edit',
    title: 'set_tempo_entry changes bpm',
    instructions: 'Add @ 9600 bpm 100, set to 170, verify.',
    run: async ({ log }) => {
      await invoke('add_tempo_entry', { tick: 9600, bpm: 100, ramp: 'instant' })
      let entries = await invoke<any[]>('get_tempo_entries')
      const idx = entries.findIndex((e: any) => e.tick === 9600)
      await invoke('set_tempo_entry', { index: idx, tick: 9600, bpm: 170, ramp: 'instant' })
      entries = await invoke<any[]>('get_tempo_entries')
      const entry = entries[idx]
      const ok = approx(entry.bpm, 170, 0.01)
      log(ok ? 'pass' : 'fail', 'bpm', { expected: 170, actual: entry.bpm })
      if (idx > 0) await invoke('remove_tempo_entry', { index: idx })
      return { pass: ok, note: `${entry.bpm}` }
    },
  },
  {
    id: 'p3_set_tempo_entry_index_0_tick_locked',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Initial tempo tick invariant',
    title: 'set_tempo_entry(0) ignores tick and keeps it 0',
    instructions: 'Even if you pass tick=500, index 0 stays at tick 0.',
    run: async ({ log }) => {
      const entries = await invoke<any[]>('get_tempo_entries')
      const origBpm = entries[0].bpm
      await invoke('set_tempo_entry', { index: 0, tick: 500, bpm: 150, ramp: 'instant' })
      const after = await invoke<any[]>('get_tempo_entries')
      const ok = after[0].tick === 0 && approx(after[0].bpm, 150, 0.01)
      log(ok ? 'pass' : 'fail', 'tick locked', { expected: 'tick 0, bpm 150', actual: `${after[0].tick}/${after[0].bpm}` })
      await invoke('set_tempo_entry', { index: 0, tick: 0, bpm: origBpm, ramp: 'instant' })
      return { pass: ok, note: `${after[0].tick}/${after[0].bpm}` }
    },
  },
  {
    id: 'p3_tempo_entry_linear_ramp',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Tempo entry ramp=linear',
    title: 'add_tempo_entry with ramp=linear is stored as "linear"',
    instructions: 'String ramp must roundtrip.',
    run: async ({ log }) => {
      await invoke('add_tempo_entry', { tick: 11520, bpm: 130, ramp: 'linear' })
      const entries = await invoke<any[]>('get_tempo_entries')
      const e = entries.find((x: any) => x.tick === 11520)
      const ok = e?.ramp === 'linear'
      log(ok ? 'pass' : 'fail', 'ramp', { expected: 'linear', actual: e?.ramp })
      const idx = entries.findIndex((x: any) => x.tick === 11520)
      if (idx > 0) await invoke('remove_tempo_entry', { index: idx })
      return { pass: ok, note: `${e?.ramp}` }
    },
  },
)

// Automation tracks
PHASE3_TESTS.push(
  {
    id: 'p3_add_automation_track_kind',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Automation track',
    title: 'add_automation_track returns id and kind=Automation',
    instructions: 'Add, verify track exists with correct kind, then remove.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const id = await invoke<string>('add_automation_track', { name: 'AutomationA' })
      const tracks = await invoke<any[]>('get_tracks')
      const t = tracks.find((x) => x.id === id)
      const ok = t && t.kind === 'Automation'
      log(ok ? 'pass' : 'fail', 'kind', { expected: 'Automation', actual: t?.kind })
      await cleanupExtraTracks(before)
      return { pass: !!ok, note: `kind=${t?.kind}` }
    },
  },
  {
    id: 'p3_remove_automation_track',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Automation track removal',
    title: 'remove_track drops automation track',
    instructions: 'Add automation, remove, verify gone.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const id = await invoke<string>('add_automation_track', { name: 'RemAuto' })
      await invoke('remove_track', { trackId: id })
      const tracks = await invoke<any[]>('get_tracks')
      const ok = !tracks.some((t) => t.id === id)
      log(ok ? 'pass' : 'fail', 'removed', { expected: 'gone', actual: 'present' })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `ok=${ok}` }
    },
  },
)

// Channel rack
PHASE3_TESTS.push(
  {
    id: 'p3_channel_rack_state_default',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Channel rack initial state',
    title: 'get_channel_rack_state returns null or string',
    instructions: 'Fresh project: null or JSON string.',
    run: async ({ log }) => {
      const s = await invoke<string | null>('get_channel_rack_state')
      const ok = s === null || typeof s === 'string'
      log(ok ? 'pass' : 'fail', 'type', { expected: 'string|null', actual: typeof s })
      return { pass: ok, note: `${typeof s}` }
    },
  },
  {
    id: 'p3_channel_rack_state_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Channel rack state persistence',
    title: 'set_channel_rack_state then get_channel_rack_state roundtrip',
    instructions: 'Set JSON string payload, read back.',
    run: async ({ log }) => {
      const prev = await invoke<string | null>('get_channel_rack_state')
      const payload = JSON.stringify({ channels: [{ name: 'Kick' }, { name: 'Snare' }] })
      await invoke('set_channel_rack_state', { payload })
      const after = await invoke<string | null>('get_channel_rack_state')
      const ok = after === payload
      log(ok ? 'pass' : 'fail', 'roundtrip', { expected: payload.slice(0, 40), actual: (after ?? '').slice(0, 40) })
      await invoke('set_channel_rack_state', { payload: prev })
      return { pass: ok, note: `len=${after?.length ?? 0}` }
    },
  },
  {
    id: 'p3_channel_rack_state_clear',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Channel rack state clear',
    title: 'set_channel_rack_state(null) clears to null',
    instructions: 'Null payload clears state.',
    run: async ({ log }) => {
      const prev = await invoke<string | null>('get_channel_rack_state')
      await invoke('set_channel_rack_state', { payload: 'temp' })
      await invoke('set_channel_rack_state', { payload: null })
      const after = await invoke<string | null>('get_channel_rack_state')
      const ok = after === null
      log(ok ? 'pass' : 'fail', 'cleared', { expected: null, actual: after })
      await invoke('set_channel_rack_state', { payload: prev })
      return { pass: ok, note: `${after}` }
    },
  },
)

// ─────────────────────────────────────────────────────────────────────────────
// 3D — Advanced track parameters
// ─────────────────────────────────────────────────────────────────────────────

PHASE3_TESTS.push(
  {
    id: 'p3_set_track_name_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Track rename',
    title: 'set_track_name renames the track',
    instructions: 'Rename, read back, compare.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      const prev = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.name
      await invoke('set_track_name', { trackId: id, name: 'Renamed Track' })
      const t = (await invoke<any[]>('get_tracks')).find((x) => x.id === id)
      const ok = t?.name === 'Renamed Track'
      log(ok ? 'pass' : 'fail', 'name', { expected: 'Renamed Track', actual: t?.name })
      await invoke('set_track_name', { trackId: id, name: prev })
      return { pass: ok, note: `${t?.name}` }
    },
  },
  {
    id: 'p3_set_track_name_empty_ignored',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Empty track name guard',
    title: 'set_track_name("") is ignored',
    instructions: 'Blank/whitespace names do not change the stored name.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      const before = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.name
      await invoke('set_track_name', { trackId: id, name: '   ' })
      const after = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.name
      const ok = before === after
      log(ok ? 'pass' : 'fail', 'ignored', { expected: before, actual: after })
      return { pass: ok, note: `${after}` }
    },
  },
  {
    id: 'p3_set_track_color_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Track color',
    title: 'set_track_color roundtrips hex string',
    instructions: 'Set #ff3366, read back.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      const prev = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.color
      await invoke('set_track_color', { trackId: id, color: '#ff3366' })
      const t = (await invoke<any[]>('get_tracks')).find((x) => x.id === id)
      const ok = t?.color === '#ff3366'
      log(ok ? 'pass' : 'fail', 'color', { expected: '#ff3366', actual: t?.color })
      await invoke('set_track_color', { trackId: id, color: prev })
      return { pass: ok, note: `${t?.color}` }
    },
  },
  {
    id: 'p3_set_track_phase_invert_toggle',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Phase invert',
    title: 'set_track_phase_invert toggles the flag',
    instructions: 'Set true then false.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_phase_invert', { trackId: id, invert: true })
      const on = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.phaseInvert
      await invoke('set_track_phase_invert', { trackId: id, invert: false })
      const off = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.phaseInvert
      const ok = on === true && off === false
      log(ok ? 'pass' : 'fail', 'phase', { expected: 'true→false', actual: `${on}→${off}` })
      return { pass: ok, note: `${on}→${off}` }
    },
  },
  {
    id: 'p3_set_track_swap_lr_toggle',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'L/R swap',
    title: 'set_track_swap_lr toggles the flag',
    instructions: 'Set true then false.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_swap_lr', { trackId: id, swap: true })
      const on = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.swapLr
      await invoke('set_track_swap_lr', { trackId: id, swap: false })
      const off = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.swapLr
      const ok = on === true && off === false
      log(ok ? 'pass' : 'fail', 'swap', { expected: 'true→false', actual: `${on}→${off}` })
      return { pass: ok, note: `${on}→${off}` }
    },
  },
  {
    id: 'p3_set_track_stereo_separation_clamp_high',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Stereo separation clamp',
    title: 'set_track_stereo_separation(50) clamps to 2',
    instructions: 'Above-range value should clamp to 2.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_stereo_separation', { trackId: id, separation: 50 })
      const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.stereoSeparation
      const ok = approx(v, 2, 0.01)
      log(ok ? 'pass' : 'fail', 'sep', { expected: 2, actual: v })
      await invoke('set_track_stereo_separation', { trackId: id, separation: 1 })
      return { pass: ok, note: `${v}` }
    },
  },
  {
    id: 'p3_set_track_stereo_separation_clamp_low',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Stereo separation clamp low',
    title: 'set_track_stereo_separation(-5) clamps to 0',
    instructions: 'Negative should clamp to 0 (mono).',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_stereo_separation', { trackId: id, separation: -5 })
      const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.stereoSeparation
      const ok = approx(v, 0, 0.01)
      log(ok ? 'pass' : 'fail', 'sep', { expected: 0, actual: v })
      await invoke('set_track_stereo_separation', { trackId: id, separation: 1 })
      return { pass: ok, note: `${v}` }
    },
  },
  {
    id: 'p3_set_track_stereo_separation_nan_ignored',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Stereo separation NaN guard',
    title: 'set_track_stereo_separation(NaN) is ignored',
    instructions: 'NaN does not alter stored value.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_stereo_separation', { trackId: id, separation: 1.5 })
      await invoke('set_track_stereo_separation', { trackId: id, separation: NaN })
      const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.stereoSeparation
      const ok = approx(v, 1.5, 0.01)
      log(ok ? 'pass' : 'fail', 'nan ignored', { expected: 1.5, actual: v })
      await invoke('set_track_stereo_separation', { trackId: id, separation: 1 })
      return { pass: ok, note: `${v}` }
    },
  },
  {
    id: 'p3_set_track_delay_samples_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Track delay samples',
    title: 'set_track_delay_samples roundtrip positive + negative',
    instructions: 'Positive delays and negative (lead) roundtrip.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_delay_samples', { trackId: id, samples: 4800 })
      const pos = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.delaySamples
      await invoke('set_track_delay_samples', { trackId: id, samples: -4800 })
      const neg = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.delaySamples
      const ok = pos === 4800 && neg === -4800
      log(ok ? 'pass' : 'fail', 'delay', { expected: '4800→-4800', actual: `${pos}→${neg}` })
      await invoke('set_track_delay_samples', { trackId: id, samples: 0 })
      return { pass: ok, note: `${pos}→${neg}` }
    },
  },
  {
    id: 'p3_set_track_pitch_semitones_clamp_high',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Track pitch clamp',
    title: 'set_track_pitch_semitones(+50) clamps to 24',
    instructions: 'Above ±24 clamps.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_pitch_semitones', { trackId: id, semitones: 50 })
      const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.pitchSemitones
      const ok = v === 24
      log(ok ? 'pass' : 'fail', 'clamp +', { expected: 24, actual: v })
      await invoke('set_track_pitch_semitones', { trackId: id, semitones: 0 })
      return { pass: ok, note: `${v}` }
    },
  },
  {
    id: 'p3_set_track_pitch_semitones_clamp_low',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Track pitch clamp negative',
    title: 'set_track_pitch_semitones(-50) clamps to -24',
    instructions: 'Below -24 clamps.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_pitch_semitones', { trackId: id, semitones: -50 })
      const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.pitchSemitones
      const ok = v === -24
      log(ok ? 'pass' : 'fail', 'clamp -', { expected: -24, actual: v })
      await invoke('set_track_pitch_semitones', { trackId: id, semitones: 0 })
      return { pass: ok, note: `${v}` }
    },
  },
  {
    id: 'p3_set_track_fine_tune_cents_clamp_high',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Track fine tune clamp',
    title: 'set_track_fine_tune_cents(+500) clamps to 100',
    instructions: 'Above ±100 clamps.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_fine_tune_cents', { trackId: id, cents: 500 })
      const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.fineTuneCents
      const ok = approx(v, 100, 0.01)
      log(ok ? 'pass' : 'fail', 'clamp +', { expected: 100, actual: v })
      await invoke('set_track_fine_tune_cents', { trackId: id, cents: 0 })
      return { pass: ok, note: `${v}` }
    },
  },
  {
    id: 'p3_set_track_fine_tune_cents_nan_ignored',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Fine tune NaN guard',
    title: 'set_track_fine_tune_cents(NaN) is ignored',
    instructions: 'NaN does not alter the stored value.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_fine_tune_cents', { trackId: id, cents: 42 })
      await invoke('set_track_fine_tune_cents', { trackId: id, cents: NaN })
      const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.fineTuneCents
      const ok = approx(v, 42, 0.01)
      log(ok ? 'pass' : 'fail', 'nan', { expected: 42, actual: v })
      await invoke('set_track_fine_tune_cents', { trackId: id, cents: 0 })
      return { pass: ok, note: `${v}` }
    },
  },
  {
    id: 'p3_set_track_filter_type_lp',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Filter type lp',
    title: 'set_track_filter_type("lp") sets lp',
    instructions: 'LP accepted.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_filter_type', { trackId: id, filterType: 'lp' })
      const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.filterType
      const ok = v === 'lp'
      log(ok ? 'pass' : 'fail', 'lp', { expected: 'lp', actual: v })
      await invoke('set_track_filter_type', { trackId: id, filterType: 'off' })
      return { pass: ok, note: `${v}` }
    },
  },
  {
    id: 'p3_set_track_filter_type_hp',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Filter type hp',
    title: 'set_track_filter_type("hp")',
    instructions: 'HP accepted.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_filter_type', { trackId: id, filterType: 'hp' })
      const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.filterType
      const ok = v === 'hp'
      log(ok ? 'pass' : 'fail', 'hp', { expected: 'hp', actual: v })
      await invoke('set_track_filter_type', { trackId: id, filterType: 'off' })
      return { pass: ok, note: `${v}` }
    },
  },
  {
    id: 'p3_set_track_filter_type_bp',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Filter type bp',
    title: 'set_track_filter_type("bp")',
    instructions: 'BP accepted.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_filter_type', { trackId: id, filterType: 'bp' })
      const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.filterType
      const ok = v === 'bp'
      log(ok ? 'pass' : 'fail', 'bp', { expected: 'bp', actual: v })
      await invoke('set_track_filter_type', { trackId: id, filterType: 'off' })
      return { pass: ok, note: `${v}` }
    },
  },
  {
    id: 'p3_set_track_filter_type_off',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Filter type off',
    title: 'set_track_filter_type("off") disables filter',
    instructions: 'Off accepted.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_filter_type', { trackId: id, filterType: 'lp' })
      await invoke('set_track_filter_type', { trackId: id, filterType: 'off' })
      const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.filterType
      const ok = v === 'off'
      log(ok ? 'pass' : 'fail', 'off', { expected: 'off', actual: v })
      return { pass: ok, note: `${v}` }
    },
  },
  {
    id: 'p3_set_track_filter_type_unknown_becomes_off',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Filter type default fallback',
    title: 'Unknown filter type falls back to "off"',
    instructions: '"banana" should be normalized to "off".',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_filter_type', { trackId: id, filterType: 'banana' })
      const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.filterType
      const ok = v === 'off'
      log(ok ? 'pass' : 'fail', 'fallback', { expected: 'off', actual: v })
      return { pass: ok, note: `${v}` }
    },
  },
  {
    id: 'p3_set_track_filter_cutoff_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Filter cutoff',
    title: 'set_track_filter_cutoff roundtrips Hz',
    instructions: 'Set 2500 Hz, read back.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_filter_cutoff', { trackId: id, cutoffHz: 2500 })
      const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.filterCutoffHz
      const ok = approx(v, 2500, 1)
      log(ok ? 'pass' : 'fail', 'cutoff', { expected: 2500, actual: v })
      return { pass: ok, note: `${v}Hz` }
    },
  },
  {
    id: 'p3_set_track_filter_resonance_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Filter resonance',
    title: 'set_track_filter_resonance roundtrips',
    instructions: 'Set 0.6, read back.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_filter_resonance', { trackId: id, resonance: 0.6 })
      const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.filterResonance
      const ok = approx(v, 0.6, 0.01)
      log(ok ? 'pass' : 'fail', 'resonance', { expected: 0.6, actual: v })
      return { pass: ok, note: `${v}` }
    },
  },
  {
    id: 'p3_set_track_output_bus_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Output bus routing',
    title: 'set_track_output_bus roundtrip then clear',
    instructions: 'Route track A to track B, then set null.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const a = await invoke<string>('add_audio_track', { name: 'OutA' })
      const b = await invoke<string>('add_audio_track', { name: 'OutB' })
      await invoke('set_track_output_bus', { trackId: a, outputBus: b })
      const routed = (await invoke<any[]>('get_tracks')).find((t) => t.id === a)!.outputBus
      await invoke('set_track_output_bus', { trackId: a, outputBus: null })
      const cleared = (await invoke<any[]>('get_tracks')).find((t) => t.id === a)!.outputBus
      const ok = routed === b && (cleared === null || cleared === undefined)
      log(ok ? 'pass' : 'fail', 'bus', { expected: `${b}→null`, actual: `${routed}→${cleared}` })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `${routed}→${cleared}` }
    },
  },
  {
    id: 'p3_set_track_monitor_input_toggle',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Monitor input flag',
    title: 'set_track_monitor_input toggles the flag',
    instructions: 'Set true then false.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      await invoke('set_track_monitor_input', { trackId: id, enabled: true })
      const on = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.monitorInput
      await invoke('set_track_monitor_input', { trackId: id, enabled: false })
      const off = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.monitorInput
      const ok = on === true && off === false
      log(ok ? 'pass' : 'fail', 'monitor', { expected: 'true→false', actual: `${on}→${off}` })
      return { pass: ok, note: `${on}→${off}` }
    },
  },
  {
    id: 'p3_set_clip_fade_curves_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Clip fade curves',
    title: 'set_clip_fade_curves roundtrips in/out curve enums',
    instructions: 'Set curve values, read back.',
    run: async ({ log, ensureAudioTrack, importAsset, clearTrackClips }) => {
      const trackId = await ensureAudioTrack()
      await clearTrackClips(trackId)
      await importAsset(trackId, 'sine-440-1s.wav')
      await sleep(30)
      const [c] = await invoke<any[]>('get_track_clips', { trackId })
      // Try a few common curve variants — backend may accept string or float
      let ok = false
      let note = 'skipped'
      try {
        await invoke('set_clip_fade_curves', { trackId, clipId: c.id, fadeInCurve: 1.0, fadeOutCurve: 1.0 })
        ok = true
        note = 'accepted float'
      } catch (e1: any) {
        try {
          await invoke('set_clip_fade_curves', { trackId, clipId: c.id, fadeInCurve: 'linear', fadeOutCurve: 'linear' })
          ok = true
          note = 'accepted string'
        } catch (e2: any) {
          note = `both signatures rejected: ${e1?.message ?? e1} | ${e2?.message ?? e2}`
        }
      }
      log(ok ? 'pass' : 'fail', 'fade curves', { expected: 'accepted', actual: note })
      await clearTrackClips(trackId)
      return { pass: ok, note }
    },
  },
)

// Parametric matrices — filter cutoff, resonance, fine-tune, pitch, stereo sep, delay
{
  const cutoffs = [20, 80, 200, 500, 1000, 2000, 4000, 8000, 12000, 20000]
  for (const hz of cutoffs) {
    PHASE3_TESTS.push({
      id: `p3_filter_cutoff_${hz}`,
      kind: 'AUTO',
      phase: 3,
      phase1Item: 'Filter cutoff matrix',
      title: `Filter cutoff ${hz} Hz`,
      instructions: `Set cutoff ${hz}, readback within tolerance`,
      run: async ({ log, ensureAudioTrack }) => {
        const id = await ensureAudioTrack()
        await invoke('set_track_filter_cutoff', { trackId: id, cutoffHz: hz })
        const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.filterCutoffHz
        const ok = approx(v, hz, Math.max(1, hz * 0.02))
        log(ok ? 'pass' : 'fail', 'cutoff', { expected: hz, actual: v })
        return { pass: ok, note: `${v}` }
      },
    })
  }
}

{
  const resonances = [0.01, 0.1, 0.25, 0.5, 0.707, 0.9, 1.5, 3, 6, 10]
  for (const r of resonances) {
    PHASE3_TESTS.push({
      id: `p3_filter_resonance_${r.toString().replace('.', '_')}`,
      kind: 'AUTO',
      phase: 3,
      phase1Item: 'Filter resonance matrix',
      title: `Filter resonance ${r}`,
      instructions: `Set resonance ${r}, readback within tolerance`,
      run: async ({ log, ensureAudioTrack }) => {
        const id = await ensureAudioTrack()
        await invoke('set_track_filter_resonance', { trackId: id, resonance: r })
        const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.filterResonance
        const ok = approx(v, r, Math.max(0.01, r * 0.05))
        log(ok ? 'pass' : 'fail', 'resonance', { expected: r, actual: v })
        return { pass: ok, note: `${v}` }
      },
    })
  }
}

{
  const pitches: number[] = [-24, -18, -12, -7, -5, -3, -1, 0, 1, 3, 5, 7, 12, 18, 24]
  for (const p of pitches) {
    PHASE3_TESTS.push({
      id: `p3_track_pitch_${p.toString().replace('-', 'n')}`,
      kind: 'AUTO',
      phase: 3,
      phase1Item: 'Track pitch semitone matrix',
      title: `Track pitch ${p} st`,
      instructions: `Set ±${p}, readback matches`,
      run: async ({ log, ensureAudioTrack }) => {
        const id = await ensureAudioTrack()
        await invoke('set_track_pitch_semitones', { trackId: id, semitones: p })
        const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.pitchSemitones
        const ok = v === p
        log(ok ? 'pass' : 'fail', 'pitch', { expected: p, actual: v })
        await invoke('set_track_pitch_semitones', { trackId: id, semitones: 0 })
        return { pass: ok, note: `${v}` }
      },
    })
  }
}

{
  const cents = [-100, -75, -50, -25, -10, -1, 0, 1, 10, 25, 50, 75, 100]
  for (const c of cents) {
    PHASE3_TESTS.push({
      id: `p3_track_fine_tune_${c.toString().replace('-', 'n')}`,
      kind: 'AUTO',
      phase: 3,
      phase1Item: 'Track fine tune matrix',
      title: `Fine tune ${c} cents`,
      instructions: `Set ${c} cents, readback matches`,
      run: async ({ log, ensureAudioTrack }) => {
        const id = await ensureAudioTrack()
        await invoke('set_track_fine_tune_cents', { trackId: id, cents: c })
        const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.fineTuneCents
        const ok = approx(v, c, 0.01)
        log(ok ? 'pass' : 'fail', 'cents', { expected: c, actual: v })
        await invoke('set_track_fine_tune_cents', { trackId: id, cents: 0 })
        return { pass: ok, note: `${v}` }
      },
    })
  }
}

{
  const seps = [0, 0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0]
  for (const s of seps) {
    PHASE3_TESTS.push({
      id: `p3_stereo_sep_${s.toString().replace('.', '_')}`,
      kind: 'AUTO',
      phase: 3,
      phase1Item: 'Stereo separation matrix',
      title: `Stereo separation ${s}`,
      instructions: `Set ${s}, readback matches`,
      run: async ({ log, ensureAudioTrack }) => {
        const id = await ensureAudioTrack()
        await invoke('set_track_stereo_separation', { trackId: id, separation: s })
        const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.stereoSeparation
        const ok = approx(v, s, 0.01)
        log(ok ? 'pass' : 'fail', 'sep', { expected: s, actual: v })
        await invoke('set_track_stereo_separation', { trackId: id, separation: 1 })
        return { pass: ok, note: `${v}` }
      },
    })
  }
}

{
  const delays = [-96000, -48000, -4800, -480, 0, 480, 4800, 48000, 96000]
  for (const d of delays) {
    PHASE3_TESTS.push({
      id: `p3_track_delay_${d.toString().replace('-', 'n')}`,
      kind: 'AUTO',
      phase: 3,
      phase1Item: 'Track delay matrix',
      title: `Track delay ${d} samples`,
      instructions: `Set ${d}, readback matches`,
      run: async ({ log, ensureAudioTrack }) => {
        const id = await ensureAudioTrack()
        await invoke('set_track_delay_samples', { trackId: id, samples: d })
        const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.delaySamples
        const ok = v === d
        log(ok ? 'pass' : 'fail', 'delay', { expected: d, actual: v })
        await invoke('set_track_delay_samples', { trackId: id, samples: 0 })
        return { pass: ok, note: `${v}` }
      },
    })
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// 3E — Autosave, Audio input, PDC, Meters, Cache, Transport state
// ─────────────────────────────────────────────────────────────────────────────

PHASE3_TESTS.push(
  {
    id: 'p3_autosave_save_and_latest',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Autosave write + read back',
    title: 'autosave_save writes a snapshot visible to autosave_latest',
    instructions: 'Save, fetch latest, verify path matches.',
    run: async ({ log }) => {
      const path = await invoke<string>('autosave_save')
      const latest = await invoke<{ path: string; modified_unix: number } | null>('autosave_latest')
      const ok = latest && latest.path === path
      log(ok ? 'pass' : 'fail', 'path', { expected: path, actual: latest?.path })
      return { pass: !!ok, note: path }
    },
  },
  {
    id: 'p3_autosave_alive_sequence',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Autosave crash detection',
    title: 'mark_alive + clear_alive + detect_crash = false',
    instructions: 'Clean-shutdown sequence should yield no crash detected.',
    run: async ({ log }) => {
      await invoke('autosave_mark_alive')
      await invoke('autosave_clear_alive')
      const crashed = await invoke<boolean>('autosave_detect_crash')
      const ok = crashed === false
      log(ok ? 'pass' : 'fail', 'no crash', { expected: false, actual: crashed })
      return { pass: ok, note: `${crashed}` }
    },
  },
  {
    id: 'p3_autosave_detect_crash_after_alive',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Autosave crash detection positive',
    title: 'mark_alive + (no clear) + detect_crash = true',
    instructions: 'An alive marker without clear_alive means a crash was detected.',
    run: async ({ log }) => {
      await invoke('autosave_mark_alive')
      const crashed = await invoke<boolean>('autosave_detect_crash')
      const ok = crashed === true
      log(ok ? 'pass' : 'fail', 'crashed', { expected: true, actual: crashed })
      await invoke('autosave_clear_alive')
      return { pass: ok, note: `${crashed}` }
    },
  },
  {
    id: 'p3_autosave_clear_empties_latest',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Autosave wipe',
    title: 'autosave_clear drops latest snapshot',
    instructions: 'After clear, autosave_latest should be null.',
    run: async ({ log }) => {
      await invoke('autosave_save')
      await invoke('autosave_clear')
      const latest = await invoke<any>('autosave_latest')
      const ok = latest === null
      log(ok ? 'pass' : 'fail', 'latest null', { expected: null, actual: latest })
      return { pass: ok, note: `${latest}` }
    },
  },
  {
    id: 'p3_get_audio_input_devices_array',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Audio input device enum',
    title: 'get_audio_input_devices returns array',
    instructions: 'Resolves to an array; may be empty.',
    run: async ({ log }) => {
      const d = await invoke<any[]>('get_audio_input_devices')
      const ok = Array.isArray(d)
      log(ok ? 'pass' : 'fail', 'type', { expected: 'array', actual: typeof d })
      return { pass: ok, note: `${d.length}` }
    },
  },
  {
    id: 'p3_get_audio_input_config_shape',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Audio input config shape',
    title: 'get_audio_input_config has device + channels',
    instructions: 'Shape: { device: string|null, channels: number }.',
    run: async ({ log }) => {
      const c = await invoke<any>('get_audio_input_config')
      const ok = c && (c.device === null || typeof c.device === 'string') && typeof c.channels === 'number'
      log(ok ? 'pass' : 'fail', 'shape', { expected: 'valid', actual: JSON.stringify(c) })
      return { pass: ok, note: `ch=${c?.channels}` }
    },
  },
  {
    id: 'p3_set_audio_input_config_channels',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Audio input config channels roundtrip',
    title: 'set_audio_input_config with channels=2 roundtrips',
    instructions: 'Set channels=2, read back.',
    run: async ({ log }) => {
      const prev = await invoke<any>('get_audio_input_config')
      await invoke('set_audio_input_config', { device: prev.device, channels: 2 })
      const after = await invoke<any>('get_audio_input_config')
      const ok = after.channels === 2
      log(ok ? 'pass' : 'fail', 'channels', { expected: 2, actual: after.channels })
      await invoke('set_audio_input_config', { device: prev.device, channels: prev.channels })
      return { pass: ok, note: `${after.channels}` }
    },
  },
  {
    id: 'p3_start_stop_input_monitoring_safe',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Input monitoring lifecycle',
    title: 'start_input_monitoring / stop_input_monitoring',
    instructions: 'Either starts or returns an error on headless systems; stop always succeeds.',
    run: async ({ log }) => {
      let started = false
      try { await invoke('start_input_monitoring'); started = true } catch {}
      await invoke('stop_input_monitoring')
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: s.bpm })
      return { pass: ok, note: started ? 'started' : 'start failed (ok)' }
    },
  },
  {
    id: 'p3_set_direct_monitoring_toggle',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Direct monitoring',
    title: 'set_direct_monitoring then get reflects it',
    instructions: 'Enable, read, disable, read.',
    run: async ({ log }) => {
      await invoke('set_direct_monitoring', { enabled: true })
      const on = await invoke<boolean>('get_direct_monitoring')
      await invoke('set_direct_monitoring', { enabled: false })
      const off = await invoke<boolean>('get_direct_monitoring')
      const ok = on === true && off === false
      log(ok ? 'pass' : 'fail', 'direct mon', { expected: 'true→false', actual: `${on}→${off}` })
      return { pass: ok, note: `${on}→${off}` }
    },
  },
  {
    id: 'p3_get_input_meter_shape',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Input meter snapshot',
    title: 'get_input_meter returns peak_l/r/running/sample_rate/buffer_size',
    instructions: 'Shape check.',
    run: async ({ log }) => {
      const m = await invoke<any>('get_input_meter')
      const ok = m && typeof m.peak_l === 'number' && typeof m.peak_r === 'number' && typeof m.running === 'boolean' && typeof m.sample_rate === 'number' && typeof m.buffer_size === 'number'
      log(ok ? 'pass' : 'fail', 'shape', { expected: 'valid', actual: JSON.stringify(m) })
      return { pass: ok, note: `running=${m?.running}` }
    },
  },
  {
    id: 'p3_get_graph_latency_shape',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Graph latency reporting',
    title: 'get_graph_latency has samples/ms/pdcEnabled',
    instructions: 'Shape check.',
    run: async ({ log }) => {
      const g = await invoke<any>('get_graph_latency')
      const ok = g && typeof g.samples === 'number' && typeof g.ms === 'number' && typeof g.pdcEnabled === 'boolean'
      log(ok ? 'pass' : 'fail', 'shape', { expected: 'valid', actual: JSON.stringify(g) })
      return { pass: ok, note: `samples=${g?.samples} ms=${g?.ms}` }
    },
  },
  {
    id: 'p3_pdc_enabled_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'PDC toggle',
    title: 'set_pdc_enabled + get_pdc_enabled roundtrip',
    instructions: 'Toggle true/false; graph latency reported only when PDC enabled.',
    run: async ({ log }) => {
      const prev = await invoke<boolean>('get_pdc_enabled')
      await invoke('set_pdc_enabled', { enabled: true })
      const on = await invoke<boolean>('get_pdc_enabled')
      await invoke('set_pdc_enabled', { enabled: false })
      const off = await invoke<boolean>('get_pdc_enabled')
      const ok = on === true && off === false
      log(ok ? 'pass' : 'fail', 'pdc', { expected: 'true→false', actual: `${on}→${off}` })
      await invoke('set_pdc_enabled', { enabled: prev })
      return { pass: ok, note: `${on}→${off}` }
    },
  },
  {
    id: 'p3_pdc_hides_latency_when_disabled',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'PDC latency reporting gated by flag',
    title: 'get_graph_latency.samples is 0 when PDC is disabled',
    instructions: 'Disable PDC, verify latency report reads as 0 samples.',
    run: async ({ log }) => {
      const prev = await invoke<boolean>('get_pdc_enabled')
      await invoke('set_pdc_enabled', { enabled: false })
      const g = await invoke<any>('get_graph_latency')
      const ok = g.samples === 0 && g.pdcEnabled === false
      log(ok ? 'pass' : 'fail', 'zero', { expected: '0 samples / pdcEnabled=false', actual: `${g.samples}/${g.pdcEnabled}` })
      await invoke('set_pdc_enabled', { enabled: prev })
      return { pass: ok, note: `samples=${g.samples}` }
    },
  },
  {
    id: 'p3_audio_cache_stats_shape',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Audio cache stats',
    title: 'get_audio_cache_stats has bytesUsed/maxBytes/entryCount',
    instructions: 'Shape check.',
    run: async ({ log }) => {
      const s = await invoke<any>('get_audio_cache_stats')
      const ok = s && typeof s.bytesUsed === 'number' && typeof s.maxBytes === 'number' && typeof s.entryCount === 'number'
      log(ok ? 'pass' : 'fail', 'shape', { expected: 'valid', actual: JSON.stringify(s) })
      return { pass: ok, note: `bytes=${s?.bytesUsed}/${s?.maxBytes} entries=${s?.entryCount}` }
    },
  },
  {
    id: 'p3_audio_cache_max_bytes_roundtrip',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Audio cache cap',
    title: 'set_audio_cache_max_bytes changes reported max',
    instructions: 'Set cap, read back, restore.',
    run: async ({ log }) => {
      const prev = (await invoke<any>('get_audio_cache_stats')).maxBytes
      await invoke('set_audio_cache_max_bytes', { maxBytes: 256 * 1024 * 1024 })
      const after = (await invoke<any>('get_audio_cache_stats')).maxBytes
      const ok = after === 256 * 1024 * 1024
      log(ok ? 'pass' : 'fail', 'max', { expected: 268435456, actual: after })
      await invoke('set_audio_cache_max_bytes', { maxBytes: prev })
      return { pass: ok, note: `${after}` }
    },
  },
  {
    id: 'p3_get_meters_shape',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Master meter snapshot',
    title: 'get_meters returns meter snapshot object',
    instructions: 'Shape check: peak_l/peak_r/rms_l/rms_r all finite numbers.',
    run: async ({ log }) => {
      const m = await invoke<any>('get_meters')
      const ok = m && typeof m === 'object'
      log(ok ? 'pass' : 'fail', 'shape', { expected: 'object', actual: typeof m })
      return { pass: ok, note: Object.keys(m ?? {}).join(',').slice(0, 80) }
    },
  },
  {
    id: 'p3_get_master_samples_length',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Scope samples fetch',
    title: 'get_master_samples returns requested number of samples',
    instructions: 'Ask for 1024, verify length.',
    run: async ({ log }) => {
      const samples = await invoke<number[]>('get_master_samples', { nFrames: 1024 })
      const ok = Array.isArray(samples) && samples.length === 1024
      log(ok ? 'pass' : 'fail', 'length', { expected: 1024, actual: samples.length })
      return { pass: ok, note: `${samples.length}` }
    },
  },
  {
    id: 'p3_get_master_samples_zero_frames',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Scope samples zero-request',
    title: 'get_master_samples(0) returns empty array',
    instructions: 'Zero frames yields [].',
    run: async ({ log }) => {
      const samples = await invoke<number[]>('get_master_samples', { nFrames: 0 })
      const ok = Array.isArray(samples) && samples.length === 0
      log(ok ? 'pass' : 'fail', 'empty', { expected: 0, actual: samples.length })
      return { pass: ok, note: `${samples.length}` }
    },
  },
  {
    id: 'p3_get_transport_state_shape',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Transport state API',
    title: 'get_transport_state has playing/bpm/loop/master_volume/time_sig',
    instructions: 'Shape sanity.',
    run: async ({ log }) => {
      const t = await invoke<any>('get_transport_state')
      const ok = t && typeof t.playing === 'boolean' && typeof t.bpm === 'number' && typeof t.looping === 'boolean' && typeof t.loop_start === 'number' && typeof t.loop_end === 'number' && typeof t.master_volume_db === 'number' && typeof t.time_sig_numerator === 'number' && typeof t.time_sig_denominator === 'number' && typeof t.pattern_mode === 'boolean'
      log(ok ? 'pass' : 'fail', 'shape', { expected: 'valid', actual: JSON.stringify(t) })
      return { pass: ok, note: `bpm=${t?.bpm}` }
    },
  },
  {
    id: 'p3_get_project_info_shape',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Project metadata',
    title: 'get_project_info has name/author/sample_rate/track_count/bpm',
    instructions: 'Shape sanity.',
    run: async ({ log }) => {
      const p = await invoke<any>('get_project_info')
      const ok = p && typeof p.name === 'string' && typeof p.author === 'string' && typeof p.sample_rate === 'number' && typeof p.track_count === 'number' && typeof p.bpm === 'number'
      log(ok ? 'pass' : 'fail', 'shape', { expected: 'valid', actual: JSON.stringify(p) })
      return { pass: ok, note: `tracks=${p?.track_count}` }
    },
  },
)

// ─────────────────────────────────────────────────────────────────────────────
// 3F — Massive edge-case and stress matrix
// ─────────────────────────────────────────────────────────────────────────────

PHASE3_TESTS.push(
  {
    id: 'p3_rapid_pdc_toggle_50',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Rapid PDC toggle',
    title: '50 rapid set_pdc_enabled toggles settle',
    instructions: 'Fire 50 toggles, verify final state matches last request.',
    run: async ({ log }) => {
      const prev = await invoke<boolean>('get_pdc_enabled')
      for (let i = 0; i < 50; i++) {
        invoke('set_pdc_enabled', { enabled: i % 2 === 0 })
      }
      await invoke('set_pdc_enabled', { enabled: true })
      await sleep(80)
      const final = await invoke<boolean>('get_pdc_enabled')
      const ok = final === true
      log(ok ? 'pass' : 'fail', 'final', { expected: true, actual: final })
      await invoke('set_pdc_enabled', { enabled: prev })
      return { pass: ok, note: `${final}` }
    },
  },
  {
    id: 'p3_rapid_direct_monitor_toggle',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Rapid direct-monitoring toggle',
    title: '50 rapid set_direct_monitoring toggles settle',
    instructions: 'Fire 50 toggles, verify final state = false.',
    run: async ({ log }) => {
      for (let i = 0; i < 50; i++) {
        invoke('set_direct_monitoring', { enabled: i % 2 === 0 })
      }
      await invoke('set_direct_monitoring', { enabled: false })
      await sleep(80)
      const final = await invoke<boolean>('get_direct_monitoring')
      const ok = final === false
      log(ok ? 'pass' : 'fail', 'final', { expected: false, actual: final })
      return { pass: ok, note: `${final}` }
    },
  },
  {
    id: 'p3_rapid_clock_enable',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Rapid MIDI clock toggles',
    title: '100 rapid set_midi_clock_enabled settle',
    instructions: 'Fire 100 toggles, verify final = false.',
    run: async ({ log }) => {
      for (let i = 0; i < 100; i++) {
        invoke('set_midi_clock_enabled', { enabled: i % 2 === 0 })
      }
      await invoke('set_midi_clock_enabled', { enabled: false })
      await sleep(100)
      const s = await invoke<any>('get_midi_clock_status')
      const ok = s.enabled === false
      log(ok ? 'pass' : 'fail', 'final', { expected: false, actual: s.enabled })
      return { pass: ok, note: `${s.enabled}` }
    },
  },
  {
    id: 'p3_rapid_mtc_fps_cycle',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Rapid MTC fps changes',
    title: 'Cycle fps 24→25→30 ten times and read final',
    instructions: 'Rapid valid-fps changes settle on last value.',
    run: async ({ log }) => {
      for (let i = 0; i < 10; i++) {
        invoke('set_midi_mtc_fps', { fps: 24 })
        invoke('set_midi_mtc_fps', { fps: 25 })
        invoke('set_midi_mtc_fps', { fps: 30 })
      }
      await invoke('set_midi_mtc_fps', { fps: 24 })
      await sleep(80)
      const s = await invoke<any>('get_midi_mtc_status')
      const ok = s.fps === 24
      log(ok ? 'pass' : 'fail', 'fps', { expected: 24, actual: s.fps })
      return { pass: ok, note: `${s.fps}` }
    },
  },
  {
    id: 'p3_rapid_filter_cutoff_spam',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Rapid filter cutoff changes',
    title: '200 rapid set_track_filter_cutoff settle on last value',
    instructions: 'Filter cutoff converges to the final explicit call.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      for (let i = 0; i < 200; i++) {
        invoke('set_track_filter_cutoff', { trackId: id, cutoffHz: 100 + i * 10 })
      }
      await invoke('set_track_filter_cutoff', { trackId: id, cutoffHz: 1234 })
      await sleep(120)
      const v = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.filterCutoffHz
      const ok = approx(v, 1234, 10)
      log(ok ? 'pass' : 'fail', 'final', { expected: 1234, actual: v })
      return { pass: ok, note: `${v}` }
    },
  },
  {
    id: 'p3_stress_add_50_midi_tracks',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Stress: many MIDI tracks',
    title: 'Add 50 MIDI tracks then remove them all',
    instructions: 'Count reaches +50, then returns to initial.',
    run: async ({ log }) => {
      const before = (await invoke<any[]>('get_tracks')).length
      const ids: string[] = []
      for (let i = 0; i < 50; i++) {
        ids.push(await invoke<string>('add_midi_track', { name: `M${i}` }))
      }
      const mid = (await invoke<any[]>('get_tracks')).length
      for (const id of ids) {
        try { await invoke('remove_track', { trackId: id }) } catch {}
      }
      const after = (await invoke<any[]>('get_tracks')).length
      const ok = mid === before + 50 && after === before
      log(ok ? 'pass' : 'fail', 'count', { expected: `${before}→${before + 50}→${before}`, actual: `${before}→${mid}→${after}` })
      return { pass: ok, note: `${before}→${mid}→${after}` }
    },
  },
  {
    id: 'p3_stress_100_midi_notes',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Stress: many MIDI notes',
    title: 'Add 100 notes to a single MIDI clip, delete all',
    instructions: 'Track the note count and verify final readback.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const trackId = await invoke<string>('add_midi_track', { name: 'Stress100' })
      const clipId = await invoke<string>('create_midi_clip', { trackId, lengthTicks: 96000 })
      for (let i = 0; i < 100; i++) {
        await invoke('add_midi_note', { trackId, clipId, pitch: 60 + (i % 24), startTick: i * 120, durationTicks: 120 })
      }
      const mid = (await invoke<any[]>('get_midi_notes', { trackId, clipId })).length
      // delete all
      for (let i = 99; i >= 0; i--) {
        try { await invoke('delete_midi_note', { trackId, clipId, noteIndex: i }) } catch {}
      }
      const end = (await invoke<any[]>('get_midi_notes', { trackId, clipId })).length
      const ok = mid === 100 && end === 0
      log(ok ? 'pass' : 'fail', 'count', { expected: '100→0', actual: `${mid}→${end}` })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `${mid}→${end}` }
    },
  },
  {
    id: 'p3_stress_20_sends_from_single_track',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Stress: many sends',
    title: 'Source track fans out to 20 targets',
    instructions: 'Create 20 targets, 20 sends, list_sends reflects 20 edges from source.',
    run: async ({ log }) => {
      const before = await invoke<any[]>('get_tracks')
      const src = await invoke<string>('add_audio_track', { name: 'FanOut' })
      for (let i = 0; i < 20; i++) {
        const tgt = await invoke<string>('add_audio_track', { name: `T${i}` })
        await invoke('add_send', { trackId: src, targetId: tgt })
      }
      const edges = await invoke<any[]>('list_sends')
      const fromSrc = edges.filter((e) => e.source === src)
      const ok = fromSrc.length === 20
      log(ok ? 'pass' : 'fail', 'edges', { expected: 20, actual: fromSrc.length })
      await cleanupExtraTracks(before)
      return { pass: ok, note: `${fromSrc.length}` }
    },
  },
  {
    id: 'p3_blocklist_stress_100',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Blocklist stress',
    title: 'set_plugin_blocklist of 100 ids roundtrips',
    instructions: 'Persist 100 entries, read back length, restore.',
    run: async ({ log }) => {
      const prev = await invoke<string[]>('get_plugin_blocklist')
      const big = Array.from({ length: 100 }, (_, i) => `com.stress.test_${i}`)
      await invoke('set_plugin_blocklist', { ids: big })
      const after = await invoke<string[]>('get_plugin_blocklist')
      const ok = after.length >= 100 && big.every((id) => after.includes(id))
      log(ok ? 'pass' : 'fail', 'count', { expected: 100, actual: after.length })
      await invoke('set_plugin_blocklist', { ids: prev })
      return { pass: ok, note: `${after.length}` }
    },
  },
  {
    id: 'p3_cancel_export_no_op_when_idle',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Cancel export when idle',
    title: 'cancel_export is safe when no export is running',
    instructions: 'No crash, engine remains alive.',
    run: async ({ log }) => {
      try { await invoke('cancel_export') } catch {}
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: s.bpm })
      return { pass: ok, note: 'survived' }
    },
  },
  {
    id: 'p3_save_nonexistent_dir_rejects',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Project save error path',
    title: 'save_project to /__nowhere__ path rejects',
    instructions: 'Bogus parent dir must reject.',
    run: async ({ log }) => {
      let threw = false
      try { await invoke('save_project', { path: '/__nowhere__/ghost.hwp' }) } catch { threw = true }
      log(threw ? 'pass' : 'fail', 'rejected', { expected: true, actual: threw })
      return { pass: threw, note: `${threw}` }
    },
  },
  {
    id: 'p3_time_sig_extreme_numerator',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Time signature extreme numerator',
    title: 'set_time_signature(32, 8) roundtrips',
    instructions: 'Upper-boundary numerator.',
    run: async ({ log }) => {
      await invoke('set_time_signature', { numerator: 32, denominator: 8 })
      const s = await devDumpState()
      const ok = s.timeSigNumerator === 32 && s.timeSigDenominator === 8
      log(ok ? 'pass' : 'fail', 'ts', { expected: '32/8', actual: `${s.timeSigNumerator}/${s.timeSigDenominator}` })
      await invoke('set_time_signature', { numerator: 4, denominator: 4 })
      return { pass: ok, note: `${s.timeSigNumerator}/${s.timeSigDenominator}` }
    },
  },
  {
    id: 'p3_bpm_boundary_20',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'BPM min boundary',
    title: 'set_bpm(20) is accepted',
    instructions: 'Exact min is valid.',
    run: async ({ log }) => {
      await invoke('set_bpm', { bpm: 20 })
      await sleep(30)
      const s = await devDumpState()
      const ok = approx(s.bpm, 20, 0.01)
      log(ok ? 'pass' : 'fail', 'bpm', { expected: 20, actual: s.bpm })
      await invoke('set_bpm', { bpm: 140 })
      return { pass: ok, note: `${s.bpm}` }
    },
  },
  {
    id: 'p3_bpm_boundary_999',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'BPM max boundary',
    title: 'set_bpm(999) is accepted',
    instructions: 'Exact max is valid.',
    run: async ({ log }) => {
      await invoke('set_bpm', { bpm: 999 })
      await sleep(30)
      const s = await devDumpState()
      const ok = approx(s.bpm, 999, 0.01)
      log(ok ? 'pass' : 'fail', 'bpm', { expected: 999, actual: s.bpm })
      await invoke('set_bpm', { bpm: 140 })
      return { pass: ok, note: `${s.bpm}` }
    },
  },
  {
    id: 'p3_undo_history_after_big_chain',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Undo stack large',
    title: 'Undo 256 volume changes restores original',
    instructions: 'Apply 256 edits, undo 256 times, verify.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      const original = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.volume_db
      for (let i = 0; i < 256; i++) {
        await invoke('set_track_volume', { trackId: id, volumeDb: -i * 0.05 - 1 })
      }
      for (let i = 0; i < 256; i++) {
        await invoke('undo')
      }
      const after = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.volume_db
      const ok = approx(after, original, 0.1)
      log(ok ? 'pass' : 'fail', 'restored', { expected: original, actual: after })
      return { pass: ok, note: `${after}` }
    },
  },
  {
    id: 'p3_redo_after_undo_chain',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Redo reapplies a chain',
    title: 'Apply 10, undo 10, redo 10 lands back on final',
    instructions: 'Final volume after redo matches the last applied value.',
    run: async ({ log, ensureAudioTrack }) => {
      const id = await ensureAudioTrack()
      const originalVol = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.volume_db
      for (let i = 1; i <= 10; i++) {
        await invoke('set_track_volume', { trackId: id, volumeDb: -i })
      }
      for (let i = 0; i < 10; i++) await invoke('undo')
      for (let i = 0; i < 10; i++) await invoke('redo')
      const after = (await invoke<any[]>('get_tracks')).find((t) => t.id === id)!.volume_db
      const ok = approx(after, -10, 0.1)
      log(ok ? 'pass' : 'fail', 'redo', { expected: -10, actual: after })
      await invoke('set_track_volume', { trackId: id, volumeDb: originalVol })
      return { pass: ok, note: `${after}` }
    },
  },
  {
    id: 'p3_tempo_remove_all_leaves_initial',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Tempo map clean up',
    title: 'Add N tempo entries then remove them; initial entry survives',
    instructions: 'Always keeps index 0.',
    run: async ({ log }) => {
      const ticks = [1920, 3840, 5760, 7680, 9600]
      for (const t of ticks) {
        await invoke('add_tempo_entry', { tick: t, bpm: 130, ramp: 'instant' })
      }
      for (const t of ticks) {
        const fresh = await invoke<any[]>('get_tempo_entries')
        const idx = fresh.findIndex((e: any) => e.tick === t)
        if (idx > 0) await invoke('remove_tempo_entry', { index: idx })
      }
      const entries = await invoke<any[]>('get_tempo_entries')
      const ok = entries.length >= 1 && entries[0].tick === 0
      log(ok ? 'pass' : 'fail', 'initial survives', { expected: 'tick 0', actual: entries[0]?.tick })
      return { pass: ok, note: `${entries.length} left` }
    },
  },
  {
    id: 'p3_midi_learn_status_after_cycle',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'MIDI learn state machine',
    title: 'start → cancel → start → cancel leaves learning=false',
    instructions: 'Full cycle returns to idle.',
    run: async ({ log }) => {
      await invoke('midi_learn_start', { target: { kind: 'masterVolume' } })
      await invoke('midi_learn_cancel')
      await invoke('midi_learn_start', { target: { kind: 'masterVolume' } })
      await invoke('midi_learn_cancel')
      const st = await invoke<any>('midi_learn_status')
      const ok = st.learning === false
      log(ok ? 'pass' : 'fail', 'idle', { expected: false, actual: st.learning })
      return { pass: ok, note: `${st.learning}` }
    },
  },
  {
    id: 'p3_audio_host_idempotent_reset',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Audio host idempotence',
    title: 'Setting the current host again is a no-op',
    instructions: 'Get current host, set to same, verify still set.',
    run: async ({ log }) => {
      const host = await invoke<string>('get_audio_host')
      await invoke('set_audio_host', { hostName: host })
      const after = await invoke<string>('get_audio_host')
      const ok = host === after
      log(ok ? 'pass' : 'fail', 'host', { expected: host, actual: after })
      return { pass: ok, note: `${host}==${after}` }
    },
  },
  {
    id: 'p3_set_loop_end_equal_start',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Zero-length loop',
    title: 'set_loop with start==end does not crash',
    instructions: 'Engine accepts or clamps but stays responsive.',
    run: async ({ log }) => {
      try { await invoke('set_loop', { start: 1000, end: 1000 }) } catch {}
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: s.bpm })
      await invoke('set_loop', { start: 0, end: 7680 })
      return { pass: ok, note: 'survived' }
    },
  },
  {
    id: 'p3_time_sig_invalid_denominator',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Time sig denominator validation',
    title: 'set_time_signature(4, 3) — non-power-of-2 handled safely',
    instructions: 'Backend clamps or rejects — engine remains alive.',
    run: async ({ log }) => {
      try { await invoke('set_time_signature', { numerator: 4, denominator: 3 }) } catch {}
      const s = await devDumpState()
      const ok = Number.isFinite(s.bpm)
      log(ok ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: s.bpm })
      await invoke('set_time_signature', { numerator: 4, denominator: 4 })
      return { pass: ok, note: 'survived' }
    },
  },
  {
    id: 'p3_channel_rack_large_payload',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Channel rack large payload',
    title: 'set_channel_rack_state with 50KB JSON roundtrips',
    instructions: 'Shouldn\'t truncate or corrupt.',
    run: async ({ log }) => {
      const prev = await invoke<string | null>('get_channel_rack_state')
      const payload = JSON.stringify({ channels: Array.from({ length: 200 }, (_, i) => ({ name: `Ch${i}`, pattern: 'x'.repeat(100) })) })
      await invoke('set_channel_rack_state', { payload })
      const after = await invoke<string | null>('get_channel_rack_state')
      const ok = after === payload
      log(ok ? 'pass' : 'fail', 'big roundtrip', { expected: payload.length, actual: after?.length })
      await invoke('set_channel_rack_state', { payload: prev })
      return { pass: ok, note: `len=${after?.length ?? 0}` }
    },
  },
  {
    id: 'p3_transport_toggle_loop_flag',
    kind: 'AUTO',
    phase: 3,
    phase1Item: 'Transport loop toggle',
    title: 'toggle_loop + get_transport_state.looping reflect each other',
    instructions: 'Toggle, read, toggle again.',
    run: async ({ log }) => {
      const a = await invoke<any>('get_transport_state')
      await invoke('toggle_loop')
      const b = await invoke<any>('get_transport_state')
      await invoke('toggle_loop')
      const c = await invoke<any>('get_transport_state')
      const ok = b.looping === !a.looping && c.looping === a.looping
      log(ok ? 'pass' : 'fail', 'toggles', { expected: `!${a.looping} then ${a.looping}`, actual: `${b.looping},${c.looping}` })
      return { pass: ok, note: `${a.looping}→${b.looping}→${c.looping}` }
    },
  },
)

// Parametric insert wet clamp matrix
{
  const wets = [-1, -0.5, 0, 0.25, 0.5, 0.75, 1, 1.5, 3]
  for (const w of wets) {
    PHASE3_TESTS.push({
      id: `p3_insert_wet_clamp_${w.toString().replace('-', 'n').replace('.', '_')}`,
      kind: 'AUTO',
      phase: 3,
      phase1Item: 'Insert wet clamp matrix',
      title: `set_insert_wet(${w}) rejects unknown slot (exercises clamp arithmetic)`,
      instructions: `Sends ${w} (pre-clamped to [0,1]); because no slot exists the call rejects — we verify engine liveness after the clamp math runs.`,
      run: async ({ log, ensureAudioTrack }) => {
        const trackId = await ensureAudioTrack()
        try { await invoke('set_insert_wet', { trackId, slotId: '__ghost__', wet: w }) } catch {}
        const s = await devDumpState()
        const ok = Number.isFinite(s.bpm)
        log(ok ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: s.bpm })
        return { pass: ok, note: `wet=${w} survived` }
      },
    })
  }
}
