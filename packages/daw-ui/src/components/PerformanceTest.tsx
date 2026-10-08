import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { hw } from '../theme'

/**
 * How many tracks this machine can play.
 *
 * The engine builds songs of test tracks beside the open project and
 * times every block at the sound card's own buffer size, doubling the
 * count until blocks run late and then narrowing down. Nothing in the
 * open song changes, and playback stops while it measures.
 */

interface Step {
  tracks: number
  p99Ms: number
  worstMs: number
  meanMs: number
  budgetMs: number
  passed: boolean
  run: 'now' | 'multicore'
}

interface Result {
  sampleRate: number
  bufferSize: number
  budgetMs: number
  threads: number
  tracks: number
  beyondTest: boolean
  multicore: { threads: number; tracks: number; beyondTest: boolean } | null
  steps: Step[]
}

interface Props {
  onClose: () => void
  /** Start a new project filled with this many test tracks. */
  onOpenTestSong: (tracks: number) => void
}

export function PerformanceTest({ onClose, onOpenTestSong }: Props) {
  const [running, setRunning] = useState(false)
  const [steps, setSteps] = useState<Step[]>([])
  const [result, setResult] = useState<Result | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [switchedOn, setSwitchedOn] = useState(false)

  useEffect(() => {
    const unlisten = listen<Step>('load-test-step', e => setSteps(s => [...s, e.payload]))
    return () => { void unlisten.then(f => f()) }
  }, [])

  const start = async () => {
    setRunning(true)
    setSteps([])
    setResult(null)
    setError(null)
    try {
      setResult(await invoke<Result>('run_load_test'))
    } catch (e) {
      setError(String(e))
    } finally {
      setRunning(false)
    }
  }

  const switchOnMulticore = async () => {
    if (!result?.multicore) return
    await invoke('set_worker_threads', { threads: result.multicore.threads })
    setSwitchedOn(true)
  }

  const latest = steps[steps.length - 1]
  const kHz = (sr: number) => `${(sr / 1000).toFixed(sr % 1000 === 0 ? 0 : 1)} kHz`

  return (
    <div
      style={{
        position: 'fixed', inset: 0, zIndex: 9800, background: 'rgba(0,0,0,0.45)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
      onMouseDown={(e) => { if (e.target === e.currentTarget && !running) onClose() }}
    >
      <div style={{
        width: 540, maxWidth: '94vw', maxHeight: '86vh',
        background: hw.bg, color: hw.textPrimary,
        border: `1px solid ${hw.border}`, borderRadius: hw.radius.lg,
        overflow: 'hidden', display: 'flex', flexDirection: 'column',
      }}>
        <div style={{
          padding: '8px 12px', display: 'flex', alignItems: 'center', gap: 12,
          background: hw.bgElevated, borderBottom: `1px solid ${hw.border}`,
        }}>
          <div style={{ fontSize: 12, fontWeight: 600 }}>Performance test</div>
          <div style={{ fontSize: 9, color: hw.textFaint }}>how many tracks this PC can play</div>
          <div style={{ flex: 1 }} />
          {running
            ? <button onClick={() => void invoke('cancel_load_test')} style={btn(false)}>Stop</button>
            : <button onClick={onClose} style={btn(false)}>Close</button>}
        </div>

        <div style={{ overflowY: 'auto', padding: 16, fontSize: 11, lineHeight: 1.6 }}>
          {!result && !running && !error && (
            <>
              <p style={{ margin: '0 0 10px', color: hw.textSecondary }}>
                The test builds songs of test tracks next to your project and times every
                block at your sound card's buffer size. It keeps adding tracks until blocks
                start arriving late, which is what you would hear as crackles. Your song is
                not changed. Playback stops while it runs, which takes one to three minutes; Stop
                ends it early.
              </p>
              <p style={{ margin: '0 0 14px', color: hw.textFaint }}>
                Each test track is a synth playing chords through an EQ, a compressor and a
                saturator.
              </p>
              <button onClick={() => void start()} style={btn(true)}>Start the test</button>
            </>
          )}

          {running && (
            <div>
              <div style={{ fontSize: 12, fontWeight: 600, marginBottom: 6 }}>
                {latest
                  ? `Trying ${latest.tracks} tracks${latest.run === 'multicore' ? ' with multi-core audio' : ''}…`
                  : 'Building the first test song…'}
              </div>
              {latest && <LoadBar step={latest} />}
              <div style={{ color: hw.textFaint, marginTop: 8 }}>
                {steps.length} {steps.length === 1 ? 'measurement' : 'measurements'} so far.
                Each one plays two seconds of the test song in real time.
              </div>
            </div>
          )}

          {error && <div style={{ color: hw.red }}>{error}</div>}

          {result && (
            <>
              <div style={{ fontSize: 15, fontWeight: 600, marginBottom: 4 }}>
                {result.tracks === 0
                  ? 'This PC could not play one test track at these settings.'
                  : result.beyondTest
                    ? `This PC plays more than ${result.tracks} test tracks.`
                    : `This PC plays about ${result.tracks} test tracks.`}
              </div>
              <div style={{ color: hw.textMuted, marginBottom: 12 }}>
                At {result.bufferSize} samples and {kHz(result.sampleRate)} ({result.budgetMs.toFixed(1)} ms
                per block), {result.threads > 0
                  ? `with ${result.threads} threads helping the audio thread.`
                  : 'on the audio thread alone.'}
              </div>

              {result.tracks === 0 && (
                <p style={{ margin: '0 0 12px', color: hw.textSecondary }}>
                  Try a larger buffer in Options &gt; Settings &gt; Audio. A larger buffer gives
                  every block more time, at the cost of a little delay when you play live.
                </p>
              )}

              {result.multicore && (
                <div style={{
                  padding: 12, marginBottom: 12, borderRadius: hw.radius.md,
                  background: 'rgba(255,255,255,0.04)', border: `1px solid ${hw.border}`,
                }}>
                  <div style={{ fontWeight: 600, marginBottom: 4 }}>
                    With multi-core audio: {result.multicore.beyondTest ? 'more than ' : 'about '}
                    {result.multicore.tracks} tracks
                  </div>
                  <div style={{ color: hw.textMuted, marginBottom: 8 }}>
                    Multi-core audio is off, so one core does all the work. Switched on, {result.multicore.threads}{' '}
                    more cores share it. The sound is the same either way.
                  </div>
                  {result.multicore.tracks > result.tracks && (
                    <button onClick={() => void switchOnMulticore()} disabled={switchedOn} style={btn(!switchedOn)}>
                      {switchedOn ? 'Multi-core audio is on' : 'Switch it on'}
                    </button>
                  )}
                </div>
              )}

              <p style={{ margin: '0 0 12px', color: hw.textFaint }}>
                Each test track is a synth playing chords through an EQ, a compressor and a
                saturator. Third-party synths are often several times heavier, so a real song
                reaches the limit sooner.
              </p>

              <details style={{ marginBottom: 14 }}>
                <summary style={{ cursor: 'pointer', color: hw.textSecondary }}>Every measurement</summary>
                <table style={{ width: '100%', marginTop: 6, borderCollapse: 'collapse', fontSize: 10 }}>
                  <thead>
                    <tr style={{ color: hw.textFaint, textAlign: 'left' }}>
                      <th style={cell}>Tracks</th>
                      <th style={cell}>Run</th>
                      <th style={cell}>Slowest 1 in 100</th>
                      <th style={cell}>Average</th>
                      <th style={cell}>Result</th>
                    </tr>
                  </thead>
                  <tbody>
                    {result.steps.map((s, i) => (
                      <tr key={i} style={{ borderTop: `1px solid ${hw.border}` }}>
                        <td style={cell}>{s.tracks}</td>
                        <td style={cell}>{s.run === 'now' ? 'now' : 'multi-core'}</td>
                        <td style={cell}>{s.p99Ms.toFixed(2)} ms</td>
                        <td style={cell}>{s.meanMs.toFixed(2)} ms</td>
                        <td style={{ ...cell, color: s.passed ? hw.green : hw.red }}>{s.passed ? 'plays' : 'late'}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </details>

              <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
                <button onClick={() => onOpenTestSong(100)} style={btn(false)}
                  title="Starts a new project with 100 test tracks, to try scrolling, the mixer and playback at that size">
                  Open a song with 100 test tracks
                </button>
                <button onClick={() => void start()} style={btn(false)}>Run again</button>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  )
}

/** How much of a block's time the latest count used. */
function LoadBar({ step }: { step: Step }) {
  const used = Math.min(1.5, step.p99Ms / step.budgetMs)
  return (
    <div>
      <div style={{ position: 'relative', height: 8, borderRadius: 4, background: 'rgba(255,255,255,0.06)', overflow: 'hidden' }}>
        <div style={{
          width: `${Math.min(100, used * 100)}%`, height: '100%',
          background: step.passed ? hw.green : hw.red, transition: 'width 0.2s',
        }} />
        <div style={{ position: 'absolute', left: '70%', top: 0, bottom: 0, width: 1, background: hw.textFaint }} />
      </div>
      <div style={{ fontSize: 10, color: hw.textFaint, marginTop: 4 }}>
        {step.tracks} tracks used {Math.round(used * 100)}% of each block's time. The line is the 70% a block may use.
      </div>
    </div>
  )
}

const cell: React.CSSProperties = { padding: '3px 6px' }

function btn(primary: boolean): React.CSSProperties {
  return {
    padding: '6px 12px', fontSize: 11, fontWeight: 600,
    background: primary ? hw.accent : 'rgba(255,255,255,0.08)',
    color: primary ? '#fff' : hw.textSecondary,
    border: 'none', borderRadius: hw.radius.sm, cursor: 'pointer', fontFamily: 'inherit',
  }
}
