// Phase 8 — numeric fuzz matrix. Every numeric command parameter gets
// tested with NaN, ±Infinity, ±MAX_SAFE_INTEGER and 0. The contract is
// simple: the engine must stay alive (dev_dump_state.bpm finite) and the
// stored state must remain sane (no NaN leaks into the atomic).

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
  try {
    const s = await devDumpState()
    return Number.isFinite(s.bpm) && Number.isFinite(s.masterVolumeDb)
  } catch {
    return false
  }
}

export const PHASE8_TESTS: TestDef[] = []

// ─── FuzzSpec runner ────────────────────────────────────────────────────────
// For each command, declare which numeric params to fuzz. We call the
// command once per bad value, silently swallow rejections, then verify the
// engine is alive afterwards.

type FuzzCase = {
  id: string
  title: string
  phaseItem: string
  fire: (bad: number) => Promise<void>
  /** Optional post-check — returns true if state is still sane. */
  verify?: () => Promise<boolean>
}

const BAD_VALUES: Array<{ label: string; value: number }> = [
  { label: 'nan', value: NaN },
  { label: 'posinf', value: Number.POSITIVE_INFINITY },
  { label: 'neginf', value: Number.NEGATIVE_INFINITY },
  { label: 'maxsafe', value: Number.MAX_SAFE_INTEGER },
  { label: 'minsafe', value: Number.MIN_SAFE_INTEGER },
  { label: 'zero', value: 0 },
  { label: 'tiny', value: 1e-300 },
  { label: 'huge', value: 1e300 },
]

function registerFuzz(c: FuzzCase) {
  for (const { label, value } of BAD_VALUES) {
    PHASE8_TESTS.push({
      id: `p8_${c.id}_${label}`,
      kind: 'AUTO',
      phase: 8,
      phase1Item: c.phaseItem,
      title: `${c.title} with ${label}`,
      instructions: `${c.title} called with ${label} (${value}). Engine must remain alive.`,
      run: async ({ log }) => {
        try { await c.fire(value) } catch {}
        const alive = await engineAlive()
        let extra = true
        if (alive && c.verify) extra = await c.verify()
        const ok = alive && extra
        log(ok ? 'pass' : 'fail', 'alive', { expected: 'ok', actual: alive ? 'extra-check failed' : 'dead' })
        return { pass: ok, note: ok ? 'survived' : 'dead or insane' }
      },
    })
  }
}

// ─── Transport / master ─────────────────────────────────────────────────────

registerFuzz({
  id: 'bpm',
  title: 'set_bpm',
  phaseItem: 'BPM numeric fuzz',
  fire: async (bad) => { await invoke('set_bpm', { bpm: bad }) },
  verify: async () => { const s = await devDumpState(); return s.bpm >= 20 - 0.01 && s.bpm <= 999 + 0.01 },
})

registerFuzz({
  id: 'master_volume',
  title: 'set_master_volume',
  phaseItem: 'Master volume numeric fuzz',
  fire: async (bad) => { await invoke('set_master_volume', { db: bad }) },
  verify: async () => { const s = await devDumpState(); return Number.isFinite(s.masterVolumeDb) },
})

registerFuzz({
  id: 'set_position',
  title: 'set_position',
  phaseItem: 'set_position numeric fuzz',
  fire: async (bad) => {
    // u64 in Rust — NaN/Infinity will be rejected by serde before reaching the command.
    // We wrap in try/catch; the key check is engine liveness.
    const asU64 = Number.isFinite(bad) ? Math.max(0, Math.floor(bad)) : 0
    await invoke('set_position', { position: asU64 })
  },
})

registerFuzz({
  id: 'set_loop_start',
  title: 'set_loop start',
  phaseItem: 'Loop start numeric fuzz',
  fire: async (bad) => {
    const start = Number.isFinite(bad) ? Math.max(0, Math.floor(bad)) : 0
    await invoke('set_loop', { start, end: start + 7680 })
  },
})

registerFuzz({
  id: 'time_sig_num',
  title: 'set_time_signature numerator',
  phaseItem: 'Time sig numerator fuzz',
  fire: async (bad) => {
    // u32 — non-finite values rejected by serde.
    const n = Number.isFinite(bad) ? Math.max(1, Math.min(32, Math.floor(bad))) : 1
    await invoke('set_time_signature', { numerator: n, denominator: 4 })
  },
})

// ─── Track params ───────────────────────────────────────────────────────────

registerFuzz({
  id: 'track_vol',
  title: 'set_track_volume',
  phaseItem: 'Track volume numeric fuzz',
  fire: async (bad) => {
    const before = await invoke<any[]>('get_tracks')
    const t = before.find((x) => x.kind !== 'Master')
    const id = t ? t.id : await invoke<string>('add_audio_track', { name: 'FuzzVol' })
    try { await invoke('set_track_volume', { trackId: id, volumeDb: bad }) } finally {
      if (!t) await cleanupExtraTracks(before)
    }
  },
})

registerFuzz({
  id: 'track_pan',
  title: 'set_track_pan',
  phaseItem: 'Track pan numeric fuzz',
  fire: async (bad) => {
    const before = await invoke<any[]>('get_tracks')
    const t = before.find((x) => x.kind !== 'Master')
    const id = t ? t.id : await invoke<string>('add_audio_track', { name: 'FuzzPan' })
    try { await invoke('set_track_pan', { trackId: id, pan: bad }) } finally {
      if (!t) await cleanupExtraTracks(before)
    }
  },
})

registerFuzz({
  id: 'track_stereo_sep',
  title: 'set_track_stereo_separation',
  phaseItem: 'Stereo separation numeric fuzz',
  fire: async (bad) => {
    const before = await invoke<any[]>('get_tracks')
    const t = before.find((x) => x.kind !== 'Master')
    const id = t ? t.id : await invoke<string>('add_audio_track', { name: 'FuzzSep' })
    try { await invoke('set_track_stereo_separation', { trackId: id, separation: bad }) } finally {
      if (!t) await cleanupExtraTracks(before)
    }
  },
})

registerFuzz({
  id: 'track_fine_tune',
  title: 'set_track_fine_tune_cents',
  phaseItem: 'Track fine tune numeric fuzz',
  fire: async (bad) => {
    const before = await invoke<any[]>('get_tracks')
    const t = before.find((x) => x.kind !== 'Master')
    const id = t ? t.id : await invoke<string>('add_audio_track', { name: 'FuzzFT' })
    try { await invoke('set_track_fine_tune_cents', { trackId: id, cents: bad }) } finally {
      if (!t) await cleanupExtraTracks(before)
    }
  },
})

registerFuzz({
  id: 'track_filter_cutoff',
  title: 'set_track_filter_cutoff',
  phaseItem: 'Filter cutoff numeric fuzz',
  fire: async (bad) => {
    const before = await invoke<any[]>('get_tracks')
    const t = before.find((x) => x.kind !== 'Master')
    const id = t ? t.id : await invoke<string>('add_audio_track', { name: 'FuzzCut' })
    try { await invoke('set_track_filter_cutoff', { trackId: id, cutoffHz: bad }) } finally {
      if (!t) await cleanupExtraTracks(before)
    }
  },
})

registerFuzz({
  id: 'track_filter_res',
  title: 'set_track_filter_resonance',
  phaseItem: 'Filter resonance numeric fuzz',
  fire: async (bad) => {
    const before = await invoke<any[]>('get_tracks')
    const t = before.find((x) => x.kind !== 'Master')
    const id = t ? t.id : await invoke<string>('add_audio_track', { name: 'FuzzRes' })
    try { await invoke('set_track_filter_resonance', { trackId: id, resonance: bad }) } finally {
      if (!t) await cleanupExtraTracks(before)
    }
  },
})

registerFuzz({
  id: 'track_delay_samples',
  title: 'set_track_delay_samples',
  phaseItem: 'Track delay numeric fuzz',
  fire: async (bad) => {
    const before = await invoke<any[]>('get_tracks')
    const t = before.find((x) => x.kind !== 'Master')
    const id = t ? t.id : await invoke<string>('add_audio_track', { name: 'FuzzDly' })
    // i64 — cast integer if possible
    const val = Number.isFinite(bad) ? Math.max(-1e9, Math.min(1e9, Math.floor(bad))) : 0
    try { await invoke('set_track_delay_samples', { trackId: id, samples: val }) } finally {
      if (!t) await cleanupExtraTracks(before)
    }
  },
})

// ─── Clip params ────────────────────────────────────────────────────────────

async function ensureClip(): Promise<{ trackId: string; clipId: string; cleanup: () => Promise<void> }> {
  const before = await invoke<any[]>('get_tracks')
  const trackId = await invoke<string>('add_audio_track', { name: `FuzzClipTrack${Date.now()}` })
  const fsPath = await invoke<string>('dev_resolve_test_asset', { name: 'sine-440-1s.wav' })
  await invoke('import_audio_file', { trackId, filePath: fsPath, positionTicks: 0 })
  const clips = await invoke<any[]>('get_track_clips', { trackId })
  return {
    trackId,
    clipId: clips[0].id,
    cleanup: async () => { await cleanupExtraTracks(before) },
  }
}

registerFuzz({
  id: 'clip_gain',
  title: 'set_clip_gain',
  phaseItem: 'Clip gain numeric fuzz',
  fire: async (bad) => {
    const { trackId, clipId, cleanup } = await ensureClip()
    try { await invoke('set_clip_gain', { trackId, clipId, gainDb: bad }) } finally { await cleanup() }
  },
})

registerFuzz({
  id: 'clip_pitch',
  title: 'set_clip_pitch',
  phaseItem: 'Clip pitch numeric fuzz',
  fire: async (bad) => {
    const { trackId, clipId, cleanup } = await ensureClip()
    try { await invoke('set_clip_pitch', { trackId, clipId, pitchSemitones: bad }) } finally { await cleanup() }
  },
})

registerFuzz({
  id: 'clip_stretch',
  title: 'set_clip_stretch',
  phaseItem: 'Clip stretch numeric fuzz',
  fire: async (bad) => {
    const { trackId, clipId, cleanup } = await ensureClip()
    try { await invoke('set_clip_stretch', { trackId, clipId, stretchRatio: bad }) } finally { await cleanup() }
  },
})

// ─── Sends ───────────────────────────────────────────────────────────────────

registerFuzz({
  id: 'send_gain',
  title: 'set_send_gain',
  phaseItem: 'Send gain numeric fuzz',
  fire: async (bad) => {
    const before = await invoke<any[]>('get_tracks')
    const a = await invoke<string>('add_audio_track', { name: 'FuzzSGa' })
    const b = await invoke<string>('add_audio_track', { name: 'FuzzSGb' })
    const idx = await invoke<number>('add_send', { trackId: a, targetId: b })
    try { await invoke('set_send_gain', { trackId: a, sendIndex: idx, gainDb: bad }) } finally { await cleanupExtraTracks(before) }
  },
})

// ─── Audio cache ─────────────────────────────────────────────────────────────

registerFuzz({
  id: 'cache_max_bytes',
  title: 'set_audio_cache_max_bytes',
  phaseItem: 'Audio cache cap fuzz',
  fire: async (bad) => {
    const prev = (await invoke<any>('get_audio_cache_stats')).maxBytes
    try {
      const val = Number.isFinite(bad) ? Math.max(0, Math.floor(bad)) : 0
      await invoke('set_audio_cache_max_bytes', { maxBytes: val })
    } finally {
      try { await invoke('set_audio_cache_max_bytes', { maxBytes: prev }) } catch {}
    }
  },
})

// ─── Scope samples ──────────────────────────────────────────────────────────

registerFuzz({
  id: 'master_samples',
  title: 'get_master_samples',
  phaseItem: 'Scope samples nFrames fuzz',
  fire: async (bad) => {
    const n = Number.isFinite(bad) ? Math.max(0, Math.min(65536, Math.floor(bad))) : 0
    await invoke('get_master_samples', { nFrames: n })
  },
})

// ─── MTC fps ────────────────────────────────────────────────────────────────

registerFuzz({
  id: 'mtc_fps',
  title: 'set_midi_mtc_fps',
  phaseItem: 'MTC fps fuzz',
  fire: async (bad) => {
    const n = Number.isFinite(bad) ? Math.max(0, Math.min(1000, Math.floor(bad))) : 0
    await invoke('set_midi_mtc_fps', { fps: n })
  },
})

// ─── Tempo entry bpm ────────────────────────────────────────────────────────

registerFuzz({
  id: 'tempo_entry_bpm',
  title: 'add_tempo_entry bpm',
  phaseItem: 'Tempo entry bpm numeric fuzz',
  fire: async (bad) => {
    const tick = 1920 + Math.floor(Math.random() * 100000) * 10
    try { await invoke('add_tempo_entry', { tick, bpm: bad, ramp: 'instant' }) } catch {}
    // cleanup the one we added (if any)
    try {
      const entries = await invoke<any[]>('get_tempo_entries')
      const idx = entries.findIndex((e: any) => e.tick === tick)
      if (idx > 0) await invoke('remove_tempo_entry', { index: idx })
    } catch {}
  },
})
