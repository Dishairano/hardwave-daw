import { invoke } from '@tauri-apps/api/core'

export interface DevTrackMeter {
  id: string
  peakLDb: number
  peakRDb: number
  rmsDb: number
}

export interface DevState {
  positionSamples: number
  playing: boolean
  recording: boolean
  looping: boolean
  loopStart: number
  loopEnd: number
  bpm: number
  masterVolumeDb: number
  timeSigNumerator: number
  timeSigDenominator: number
  timeSigPacked: number
  patternMode: boolean
  activeDeviceName: string | null
  selectedDeviceName: string | null
  sampleRate: number
  bufferSize: number
  streamRunning: boolean
  streamErrorFlag: boolean
  masterPeakDb: number
  masterPeakHoldDb: number
  masterRmsDb: number
  masterTruePeakDb: number
  masterClipped: boolean
  tracks: DevTrackMeter[]
}

// Tauri returns snake_case by default; rename keys here.
function fromSnake(raw: any): DevState {
  return {
    positionSamples: raw.position_samples,
    playing: raw.playing,
    recording: raw.recording,
    looping: raw.looping,
    loopStart: raw.loop_start,
    loopEnd: raw.loop_end,
    bpm: raw.bpm,
    masterVolumeDb: raw.master_volume_db,
    timeSigNumerator: raw.time_sig_numerator,
    timeSigDenominator: raw.time_sig_denominator,
    timeSigPacked: raw.time_sig_packed,
    patternMode: raw.pattern_mode,
    activeDeviceName: raw.active_device_name,
    selectedDeviceName: raw.selected_device_name,
    sampleRate: raw.sample_rate,
    bufferSize: raw.buffer_size,
    streamRunning: raw.stream_running,
    streamErrorFlag: raw.stream_error_flag,
    masterPeakDb: raw.master_peak_db,
    masterPeakHoldDb: raw.master_peak_hold_db,
    masterRmsDb: raw.master_rms_db,
    masterTruePeakDb: raw.master_true_peak_db,
    masterClipped: raw.master_clipped,
    tracks: (raw.tracks ?? []).map((t: any) => ({
      id: t.id,
      peakLDb: t.peak_l_db,
      peakRDb: t.peak_r_db,
      rmsDb: t.rms_db,
    })),
  }
}

export async function devDumpState(): Promise<DevState> {
  const raw = await invoke<any>('dev_dump_state')
  return fromSnake(raw)
}

export async function devForceDeviceError(): Promise<void> {
  await invoke('dev_force_device_error')
}

export async function devListTestAssets(): Promise<string[]> {
  return invoke<string[]>('dev_list_test_assets')
}

export async function devResolveTestAsset(name: string): Promise<string> {
  return invoke<string>('dev_resolve_test_asset', { name })
}

/** Simulate a keyboard event on the window. */
export function simulateKey(code: string, opts?: { ctrlKey?: boolean; shiftKey?: boolean; metaKey?: boolean }) {
  const event = new KeyboardEvent('keydown', {
    code,
    key: code.replace('Key', '').toLowerCase(),
    bubbles: true,
    cancelable: true,
    ctrlKey: opts?.ctrlKey ?? false,
    shiftKey: opts?.shiftKey ?? false,
    metaKey: opts?.metaKey ?? false,
  })
  window.dispatchEvent(event)
}

/** Query a DOM element by data-testid. */
export function queryTestId(id: string): HTMLElement | null {
  return document.querySelector(`[data-testid="${id}"]`)
}

/** Get the numeric data-db attribute from a meter element. */
export function getMeterDb(testId: string): number | null {
  const el = queryTestId(testId)
  if (!el) return null
  const val = el.getAttribute('data-db')
  return val !== null ? parseFloat(val) : null
}

/** Capture a screenshot of the entire window as a base64 PNG data URL. */
export async function captureScreenshot(): Promise<string> {
  const canvas = document.createElement('canvas')
  const rect = document.documentElement.getBoundingClientRect()
  canvas.width = window.innerWidth
  canvas.height = window.innerHeight
  const ctx = canvas.getContext('2d')!

  // Use html2canvas-style approach: draw the document to an offscreen canvas
  // via SVG foreignObject serialization
  const svgData = `
    <svg xmlns="http://www.w3.org/2000/svg" width="${canvas.width}" height="${canvas.height}">
      <foreignObject width="100%" height="100%">
        ${new XMLSerializer().serializeToString(document.documentElement)}
      </foreignObject>
    </svg>`
  const img = new Image()
  const blob = new Blob([svgData], { type: 'image/svg+xml;charset=utf-8' })
  const url = URL.createObjectURL(blob)

  return new Promise((resolve) => {
    img.onload = () => {
      ctx.drawImage(img, 0, 0)
      URL.revokeObjectURL(url)
      resolve(canvas.toDataURL('image/png'))
    }
    img.onerror = () => {
      URL.revokeObjectURL(url)
      resolve('')
    }
    img.src = url
  })
}

/** Capture a specific element's bounding rect as a cropped PNG data URL. */
export async function captureElement(testId: string): Promise<string> {
  const el = queryTestId(testId)
  if (!el) return ''
  const rect = el.getBoundingClientRect()
  const full = await captureScreenshot()
  if (!full) return ''

  const img = new Image()
  return new Promise((resolve) => {
    img.onload = () => {
      const canvas = document.createElement('canvas')
      canvas.width = rect.width * devicePixelRatio
      canvas.height = rect.height * devicePixelRatio
      const ctx = canvas.getContext('2d')!
      ctx.drawImage(img,
        rect.x * devicePixelRatio, rect.y * devicePixelRatio,
        rect.width * devicePixelRatio, rect.height * devicePixelRatio,
        0, 0, canvas.width, canvas.height)
      resolve(canvas.toDataURL('image/png'))
    }
    img.onerror = () => resolve('')
    img.src = full
  })
}

/** Simulate a mouse click at (x, y) on a canvas element. */
export function clickCanvas(testId: string, x: number, y: number) {
  const canvas = queryTestId(testId) as HTMLCanvasElement | null
  if (!canvas) return false
  const rect = canvas.getBoundingClientRect()
  const evt = new MouseEvent('mousedown', {
    clientX: rect.left + x,
    clientY: rect.top + y,
    bubbles: true,
  })
  canvas.dispatchEvent(evt)
  const up = new MouseEvent('mouseup', {
    clientX: rect.left + x,
    clientY: rect.top + y,
    bubbles: true,
  })
  canvas.dispatchEvent(up)
  return true
}
