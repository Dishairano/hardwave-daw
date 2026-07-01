/**
 * Dev-only Tauri IPC mock for the screenshot harness (screenshot.html).
 *
 * `@tauri-apps/api/core` `invoke()` reads `window.__TAURI_INTERNALS__.invoke`
 * lazily at call time, so installing this on import — before the harness
 * mounts and effects fire — is enough to satisfy the handful of commands the
 * Toolbar + Arrangement poll. NOT part of the shipped app: only screenshot.tsx
 * imports it, and only screenshot.html loads that entry (dev server only).
 */

// Synthetic waveform peaks: [min, max, rms, brightness]. Models a kick-ish
// hit — loud decaying body (bass → red) with a bright transient at the
// onset (treble → blue) — so the spectral colouring is visible in a shot.
function synthPeaks(n: number): [number, number, number, number][] {
  const out: [number, number, number, number][] = []
  for (let i = 0; i < n; i++) {
    const t = i / Math.max(1, n)
    const env = Math.exp(-t * 4) * (0.35 + 0.65 * Math.abs(Math.sin(t * 30)))
    const max = env
    const min = -env * 0.92
    const rms = env * 0.6
    // Bright (blue) at the very onset, settling into bass (red) body.
    const brightness = Math.min(1, 0.9 * Math.exp(-t * 26) + 0.12)
    out.push([min, max, rms, brightness])
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
