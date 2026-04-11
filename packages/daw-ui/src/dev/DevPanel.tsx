// Dev Panel — stripped before merge to master.
// Floating overlay for Phase 1 feature verification.

import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { hw } from '../theme'
import { TESTS, type TestDef, type TestStatus, type TestRunContext } from './tests'
import { useLogStore, type LogLevel } from './logStore'
import { devDumpState, devResolveTestAsset, type DevState } from './devApi'

// ─── Styles ───────────────────────────────────────────────────────────────────

const PANEL: React.CSSProperties = {
  position: 'fixed',
  top: 40,
  right: 8,
  width: 540,
  maxHeight: 'calc(100vh - 56px)',
  background: 'rgba(8,8,12,0.96)',
  border: `1px solid ${hw.border}`,
  borderRadius: hw.radius.lg,
  backdropFilter: hw.blur.md,
  display: 'flex',
  flexDirection: 'column',
  zIndex: 9999,
  fontFamily: 'JetBrains Mono, Consolas, monospace',
  fontSize: 11,
  color: hw.textPrimary,
  boxShadow: '0 8px 32px rgba(0,0,0,0.6)',
  overflow: 'hidden',
}

const HEADER: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  padding: '8px 12px',
  background: 'rgba(255,255,255,0.03)',
  borderBottom: `1px solid ${hw.border}`,
  fontWeight: 600,
  fontSize: 12,
  userSelect: 'none',
}

const TABS: React.CSSProperties = {
  display: 'flex',
  gap: 0,
  borderBottom: `1px solid ${hw.border}`,
}

const SCROLL: React.CSSProperties = {
  flex: 1,
  overflowY: 'auto',
  overflowX: 'hidden',
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

const statusColor: Record<TestStatus, string> = {
  idle: hw.textFaint,
  running: hw.yellow,
  pass: hw.green,
  fail: hw.red,
}

const statusLabel: Record<TestStatus, string> = {
  idle: '---',
  running: 'RUN',
  pass: 'PASS',
  fail: 'FAIL',
}

// ─── Tab button ───────────────────────────────────────────────────────────────

function Tab({ label, active, onClick }: { label: string; active: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      style={{
        flex: 1,
        padding: '6px 0',
        background: active ? 'rgba(255,255,255,0.06)' : 'transparent',
        border: 'none',
        borderBottom: active ? `2px solid ${hw.accent}` : '2px solid transparent',
        color: active ? hw.textPrimary : hw.textMuted,
        fontSize: 11,
        cursor: 'pointer',
        fontFamily: 'inherit',
      }}
    >
      {label}
    </button>
  )
}

// ─── Test row ─────────────────────────────────────────────────────────────────

function TestRow({
  test,
  status,
  onRun,
  onManualResult,
}: {
  test: TestDef
  status: TestStatus
  onRun: () => void
  onManualResult: (pass: boolean) => void
}) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        padding: '6px 12px',
        borderBottom: `1px solid ${hw.borderDark}`,
      }}
    >
      <span
        style={{
          width: 36,
          textAlign: 'center',
          color: statusColor[status],
          fontWeight: 700,
          fontSize: 10,
          flexShrink: 0,
        }}
      >
        {statusLabel[status]}
      </span>
      <span
        style={{
          fontSize: 9,
          color: test.kind === 'AUTO' ? hw.blue : hw.yellow,
          width: 44,
          flexShrink: 0,
        }}
      >
        {test.kind}
      </span>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontWeight: 500, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
          {test.title}
        </div>
        <div style={{ fontSize: 10, color: hw.textMuted, marginTop: 1, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
          {test.instructions}
        </div>
      </div>
      {test.kind === 'AUTO' && test.run ? (
        <button
          disabled={status === 'running'}
          onClick={onRun}
          style={{
            padding: '3px 10px',
            background: status === 'running' ? hw.bgElevated : hw.accent,
            border: 'none',
            borderRadius: 4,
            color: '#fff',
            fontSize: 10,
            cursor: status === 'running' ? 'wait' : 'pointer',
            fontFamily: 'inherit',
            flexShrink: 0,
          }}
        >
          {status === 'running' ? '...' : 'Run'}
        </button>
      ) : (
        <div style={{ display: 'flex', gap: 4, flexShrink: 0 }}>
          <button
            onClick={() => onManualResult(true)}
            style={{
              padding: '3px 8px',
              background: status === 'pass' ? hw.green : hw.bgElevated,
              border: 'none',
              borderRadius: 4,
              color: '#fff',
              fontSize: 10,
              cursor: 'pointer',
              fontFamily: 'inherit',
            }}
          >
            Pass
          </button>
          <button
            onClick={() => onManualResult(false)}
            style={{
              padding: '3px 8px',
              background: status === 'fail' ? hw.red : hw.bgElevated,
              border: 'none',
              borderRadius: 4,
              color: '#fff',
              fontSize: 10,
              cursor: 'pointer',
              fontFamily: 'inherit',
            }}
          >
            Fail
          </button>
        </div>
      )}
    </div>
  )
}

// ─── State dump ───────────────────────────────────────────────────────────────

function StateDump() {
  const [state, setState] = useState<DevState | null>(null)

  useEffect(() => {
    let mounted = true
    const tick = async () => {
      try {
        const s = await devDumpState()
        if (mounted) setState(s)
      } catch {}
    }
    tick()
    const id = setInterval(tick, 200)
    return () => { mounted = false; clearInterval(id) }
  }, [])

  if (!state) return <div style={{ padding: 12, color: hw.textMuted }}>Loading state...</div>

  const rows: [string, unknown][] = [
    ['position_samples', state.positionSamples],
    ['playing', state.playing],
    ['recording', state.recording],
    ['looping', state.looping],
    ['loop_start', state.loopStart],
    ['loop_end', state.loopEnd],
    ['bpm', state.bpm],
    ['master_volume_db', state.masterVolumeDb.toFixed(2)],
    ['time_sig', `${state.timeSigNumerator}/${state.timeSigDenominator} (packed: ${state.timeSigPacked})`],
    ['pattern_mode', state.patternMode],
    ['---', '--- AUDIO DEVICE ---'],
    ['active_device', state.activeDeviceName ?? '(none)'],
    ['selected_device', state.selectedDeviceName ?? '(default)'],
    ['sample_rate', state.sampleRate],
    ['buffer_size', state.bufferSize],
    ['stream_running', state.streamRunning],
    ['stream_error_flag', state.streamErrorFlag],
    ['---', '--- MASTER METER ---'],
    ['peak_db', state.masterPeakDb.toFixed(2)],
    ['peak_hold_db', state.masterPeakHoldDb.toFixed(2)],
    ['rms_db', state.masterRmsDb.toFixed(2)],
    ['true_peak_db', state.masterTruePeakDb.toFixed(2)],
    ['clipped', state.masterClipped],
  ]

  for (const t of state.tracks) {
    rows.push([`--- TRACK ${t.id.slice(0, 8)}`, '---'])
    rows.push([`  peak_l_db`, t.peakLDb.toFixed(2)])
    rows.push([`  peak_r_db`, t.peakRDb.toFixed(2)])
    rows.push([`  rms_db`, t.rmsDb.toFixed(2)])
  }

  return (
    <div style={{ padding: '6px 12px' }}>
      {rows.map(([k, v], i) => (
        <div key={i} style={{ display: 'flex', gap: 8, lineHeight: '18px' }}>
          <span style={{ color: hw.textMuted, width: 160, flexShrink: 0 }}>{String(k)}</span>
          <span style={{ color: typeof v === 'boolean' ? (v ? hw.green : hw.red) : hw.textPrimary }}>
            {String(v)}
          </span>
        </div>
      ))}
    </div>
  )
}

// ─── Event log ────────────────────────────────────────────────────────────────

function EventLog() {
  const entries = useLogStore((s) => s.entries)
  const endRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [entries.length])

  const levelColor: Record<LogLevel, string> = {
    info: hw.textMuted,
    pass: hw.green,
    fail: hw.red,
    event: hw.blue,
  }

  return (
    <div style={{ padding: '4px 12px' }}>
      {entries.slice(-500).map((e, i) => {
        const ts = new Date(e.ts).toISOString().slice(11, 23)
        return (
          <div key={i} style={{ lineHeight: '16px', whiteSpace: 'pre-wrap', wordBreak: 'break-all' }}>
            <span style={{ color: hw.textFaint }}>{ts} </span>
            <span style={{ color: levelColor[e.level], fontWeight: 600 }}>{e.level.toUpperCase().padEnd(5)} </span>
            {e.test && <span style={{ color: hw.yellow }}>[{e.test}] </span>}
            <span>{e.message}</span>
            {e.expected !== undefined && (
              <span style={{ color: hw.textMuted }}> | expected={JSON.stringify(e.expected)}</span>
            )}
            {e.actual !== undefined && (
              <span style={{ color: hw.textMuted }}> actual={JSON.stringify(e.actual)}</span>
            )}
          </div>
        )
      })}
      <div ref={endRef} />
    </div>
  )
}

// ─── Asset loader ─────────────────────────────────────────────────────────────

function AssetLoader() {
  const append = useLogStore((s) => s.append)
  const [loading, setLoading] = useState<string | null>(null)

  const assets = [
    { name: 'sine_1khz_-6dbfs_stereo_5s.wav', desc: '1 kHz sine -6 dBFS (level cal)' },
    { name: 'pink_noise_-12dbfs_10s.wav', desc: 'Pink noise -12 dBFS (meter test)' },
    { name: 'tone_burst_silence.wav', desc: 'Tone burst 1s + 4s silence (peak hold)' },
    { name: 'stereo_pan_test.wav', desc: 'L:1kHz R:500Hz (stereo independence)' },
  ]

  const load = async (name: string) => {
    setLoading(name)
    try {
      // Ensure we have a track
      const tracks = await invoke<any[]>('get_tracks')
      let trackId: string
      if (tracks.length === 0) {
        trackId = await invoke<string>('add_audio_track', { name: 'Test Track' })
      } else {
        trackId = tracks[0].id
      }
      // Resolve bundled asset to fs path
      const fsPath = await devResolveTestAsset(name)
      append({ level: 'info', message: `Loading ${name} → track ${trackId.slice(0, 8)}...` })
      await invoke('import_audio_file', { trackId, filePath: fsPath, positionTicks: 0 })
      append({ level: 'pass', message: `Loaded ${name}` })
    } catch (e: any) {
      append({ level: 'fail', message: `Failed to load ${name}: ${e}` })
    } finally {
      setLoading(null)
    }
  }

  return (
    <div style={{ padding: '8px 12px' }}>
      <div style={{ color: hw.textMuted, marginBottom: 6, fontSize: 10 }}>
        Load bundled test WAVs onto a track for manual listening.
      </div>
      {assets.map((a) => (
        <div key={a.name} style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
          <button
            disabled={loading !== null}
            onClick={() => load(a.name)}
            style={{
              padding: '3px 10px',
              background: loading === a.name ? hw.bgElevated : hw.accent,
              border: 'none',
              borderRadius: 4,
              color: '#fff',
              fontSize: 10,
              cursor: loading ? 'wait' : 'pointer',
              fontFamily: 'inherit',
              flexShrink: 0,
            }}
          >
            {loading === a.name ? '...' : 'Load'}
          </button>
          <span style={{ fontSize: 10 }}>{a.desc}</span>
        </div>
      ))}
    </div>
  )
}

// ─── Main panel ───────────────────────────────────────────────────────────────

export function DevPanel({ onClose }: { onClose: () => void }) {
  const [tab, setTab] = useState<'tests' | 'state' | 'log' | 'assets'>('tests')
  const [statuses, setStatuses] = useState<Record<string, TestStatus>>({})
  const append = useLogStore((s) => s.append)
  const exportText = useLogStore((s) => s.exportText)
  const clearLog = useLogStore((s) => s.clear)

  // Subscribe to daw:* events for the event log
  useEffect(() => {
    const unsubs: (() => void)[] = []
    const events = ['daw:transport', 'daw:meters', 'daw:trackMeters']
    // Only log at reduced rate to avoid flooding
    let lastEventLog = 0
    for (const name of events) {
      listen(name, (e) => {
        const now = Date.now()
        // Log daw:transport every 1s; skip meters events in the log (too noisy)
        if (name === 'daw:transport' && now - lastEventLog > 1000) {
          lastEventLog = now
          append({ level: 'event', message: `${name}: ${JSON.stringify(e.payload)}` })
        }
      }).then((unsub) => unsubs.push(unsub))
    }
    return () => unsubs.forEach((u) => u())
  }, [])

  const makeContext = useCallback(
    (testId: string): TestRunContext => ({
      log: (level, message, extra) => {
        append({ level, test: testId, message, ...extra })
      },
      ensureAudioTrack: async () => {
        const tracks = await invoke<any[]>('get_tracks')
        if (tracks.length > 0) return tracks[0].id
        return invoke<string>('add_audio_track', { name: 'Test Track' })
      },
      importAsset: async (trackId, assetName) => {
        const fsPath = await devResolveTestAsset(assetName)
        await invoke('import_audio_file', { trackId, filePath: fsPath, positionTicks: 0 })
      },
      clearTrackClips: async (trackId) => {
        const clips = await invoke<any[]>('get_track_clips', { trackId })
        for (const c of clips) {
          await invoke('delete_clip', { trackId, clipId: c.id })
        }
      },
    }),
    [],
  )

  const runTest = useCallback(
    async (test: TestDef) => {
      if (!test.run) return
      setStatuses((s) => ({ ...s, [test.id]: 'running' }))
      append({ level: 'info', test: test.id, message: `Starting: ${test.title}` })
      try {
        const ctx = makeContext(test.id)
        const { pass, note } = await test.run(ctx)
        setStatuses((s) => ({ ...s, [test.id]: pass ? 'pass' : 'fail' }))
        append({ level: pass ? 'pass' : 'fail', test: test.id, message: `Result: ${note}` })
      } catch (e: any) {
        setStatuses((s) => ({ ...s, [test.id]: 'fail' }))
        append({ level: 'fail', test: test.id, message: `Exception: ${e?.message ?? e}` })
      }
    },
    [makeContext],
  )

  const manualResult = useCallback(
    (test: TestDef, pass: boolean) => {
      setStatuses((s) => ({ ...s, [test.id]: pass ? 'pass' : 'fail' }))
      append({
        level: pass ? 'pass' : 'fail',
        test: test.id,
        message: `Manual: ${pass ? 'PASS' : 'FAIL'} — ${test.title}`,
      })
    },
    [],
  )

  const runAllAuto = useCallback(async () => {
    for (const test of TESTS) {
      if (test.kind === 'AUTO' && test.run) {
        await runTest(test)
      }
    }
  }, [runTest])

  const copyLog = useCallback(() => {
    const text = exportText()
    navigator.clipboard.writeText(text).then(
      () => append({ level: 'info', message: 'Log copied to clipboard' }),
      () => append({ level: 'fail', message: 'Failed to copy to clipboard' }),
    )
  }, [exportText])

  // Summary counts
  const passed = TESTS.filter((t) => statuses[t.id] === 'pass').length
  const failed = TESTS.filter((t) => statuses[t.id] === 'fail').length
  const total = TESTS.length

  return (
    <div style={PANEL}>
      <div style={HEADER}>
        <span>Dev Panel — Phase 1 Verification</span>
        <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          <span style={{ color: hw.green, fontSize: 10 }}>{passed}P</span>
          <span style={{ color: hw.red, fontSize: 10 }}>{failed}F</span>
          <span style={{ color: hw.textMuted, fontSize: 10 }}>/{total}</span>
          <button
            onClick={onClose}
            style={{
              background: 'none',
              border: 'none',
              color: hw.textMuted,
              cursor: 'pointer',
              fontSize: 14,
              padding: '0 4px',
            }}
          >
            x
          </button>
        </div>
      </div>

      <div style={TABS}>
        <Tab label="Tests" active={tab === 'tests'} onClick={() => setTab('tests')} />
        <Tab label="State" active={tab === 'state'} onClick={() => setTab('state')} />
        <Tab label="Log" active={tab === 'log'} onClick={() => setTab('log')} />
        <Tab label="Assets" active={tab === 'assets'} onClick={() => setTab('assets')} />
      </div>

      {/* Action bar */}
      <div style={{ display: 'flex', gap: 6, padding: '6px 12px', borderBottom: `1px solid ${hw.borderDark}` }}>
        <button
          onClick={runAllAuto}
          style={{
            padding: '3px 12px',
            background: hw.accent,
            border: 'none',
            borderRadius: 4,
            color: '#fff',
            fontSize: 10,
            cursor: 'pointer',
            fontFamily: 'inherit',
          }}
        >
          Run All AUTO
        </button>
        <button
          onClick={copyLog}
          style={{
            padding: '3px 12px',
            background: hw.bgElevated,
            border: `1px solid ${hw.border}`,
            borderRadius: 4,
            color: hw.textSecondary,
            fontSize: 10,
            cursor: 'pointer',
            fontFamily: 'inherit',
          }}
        >
          Copy Log
        </button>
        <button
          onClick={clearLog}
          style={{
            padding: '3px 12px',
            background: hw.bgElevated,
            border: `1px solid ${hw.border}`,
            borderRadius: 4,
            color: hw.textSecondary,
            fontSize: 10,
            cursor: 'pointer',
            fontFamily: 'inherit',
          }}
        >
          Clear
        </button>
      </div>

      <div style={SCROLL}>
        {tab === 'tests' &&
          TESTS.map((t) => (
            <TestRow
              key={t.id}
              test={t}
              status={statuses[t.id] ?? 'idle'}
              onRun={() => runTest(t)}
              onManualResult={(pass) => manualResult(t, pass)}
            />
          ))}
        {tab === 'state' && <StateDump />}
        {tab === 'log' && <EventLog />}
        {tab === 'assets' && <AssetLoader />}
      </div>
    </div>
  )
}
