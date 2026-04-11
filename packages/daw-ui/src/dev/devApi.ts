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
