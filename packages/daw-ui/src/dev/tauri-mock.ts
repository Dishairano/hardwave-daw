/**
 * Dev-only Tauri IPC mock for the screenshot harness (screenshot.html).
 *
 * `@tauri-apps/api/core` `invoke()` reads `window.__TAURI_INTERNALS__.invoke`
 * lazily at call time, so installing this on import — before the harness
 * mounts and effects fire — is enough to satisfy the handful of commands the
 * Toolbar + Arrangement poll. NOT part of the shipped app: only screenshot.tsx
 * imports it, and only screenshot.html loads that entry (dev server only).
 */

// Synthetic waveform peaks: [min, max, rms, brightness]. Models a realistic
// drum-loop-ish signal — several transient hits, each a bright onset (treble
// → blue) decaying into a bass body (red), with per-sample jitter so the
// waveform has the jagged detail of real audio rather than a smooth blob.
function synthPeaks(n: number): [number, number, number, number][] {
  // Deterministic pseudo-random so screenshots are reproducible.
  const rand = (i: number) => {
    const x = Math.sin(i * 127.1 + 311.7) * 43758.5453
    return x - Math.floor(x)
  }
  const hits = [0.0, 0.18, 0.31, 0.5, 0.63, 0.75, 0.88]
  const out: [number, number, number, number][] = []
  for (let i = 0; i < n; i++) {
    const t = i / Math.max(1, n)
    // Nearest preceding hit → transient envelope + brightness.
    let env = 0
    let bright = 0.08
    for (const hp of hits) {
      const d = t - hp
      if (d >= 0) {
        const e = Math.exp(-d * 26)
        if (e > env) {
          env = e
          bright = Math.min(1, e * 0.95 + 0.08) // bright at onset, decays to bass
        }
      }
    }
    const jitter = 0.45 + 0.55 * rand(i) // per-bucket detail
    const amp = Math.min(1, env * jitter)
    const max = amp
    const min = -amp * (0.82 + 0.18 * rand(i + 7))
    const rms = amp * (0.5 + 0.35 * rand(i + 13))
    out.push([min, max, rms, bright])
  }
  return out
}

interface TauriInternals {
  transformCallback: (cb: unknown) => unknown
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>
}

const mock: TauriInternals = {
  transformCallback: (cb) => cb,
  invoke: async (cmd, args) => {
    switch (cmd) {
      case 'get_waveform_peaks':
        return synthPeaks((args?.numBuckets as number) ?? 256)
      case 'get_graph_latency':
        return { samples: 0, ms: 0, pdcEnabled: true }
      case 'get_midi_activity':
        return { open_ports: [], ms_since_last_event: null }
      // Event plugin — let listeners register harmlessly.
      case 'plugin:event|listen':
        return 0
      case 'plugin:event|unlisten':
        return null
      default:
        return null
    }
  },
}

;(window as unknown as { __TAURI_INTERNALS__: TauriInternals }).__TAURI_INTERNALS__ = mock
