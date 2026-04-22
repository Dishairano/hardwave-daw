import { useState, useEffect, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../../theme'

interface AudioDevice {
  name: string
  is_default: boolean
  sample_rates: number[]
  max_channels: number
}

interface AudioConfig {
  device: string | null
  sample_rate: number
  buffer_size: number
}

interface AudioInputConfig {
  device: string | null
  channels: number
}

interface WasapiExclusiveStatus {
  enabled: boolean
  available: boolean
}

interface InputMeterSnapshot {
  peak_l: number
  peak_r: number
  running: boolean
  sample_rate: number
  buffer_size: number
}

interface AudioSettingsProps {
  onClose: () => void
}

const BUFFER_SIZES = [64, 128, 256, 512, 1024, 2048, 4096]

export function AudioSettings({ onClose }: AudioSettingsProps) {
  const [devices, setDevices] = useState<AudioDevice[]>([])
  const [inputDevices, setInputDevices] = useState<AudioDevice[]>([])
  const [config, setConfig] = useState<AudioConfig>({ device: null, sample_rate: 48000, buffer_size: 512 })
  const [inputConfig, setInputConfig] = useState<AudioInputConfig>({ device: null, channels: 2 })
  const [selectedDevice, setSelectedDevice] = useState<string | null>(null)
  const [selectedInput, setSelectedInput] = useState<string | null>(null)
  const [selectedInputChannels, setSelectedInputChannels] = useState(2)
  const [selectedRate, setSelectedRate] = useState(48000)
  const [selectedBuffer, setSelectedBuffer] = useState(512)
  const [applying, setApplying] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [exclusive, setExclusive] = useState<WasapiExclusiveStatus>({ enabled: false, available: false })
  const [monitor, setMonitor] = useState<InputMeterSnapshot | null>(null)
  const [monitorOn, setMonitorOn] = useState(false)
  const peakDecayRef = useRef({ l: 0, r: 0 })
  const [midiPorts, setMidiPorts] = useState<string[]>([])
  const [midiOpen, setMidiOpen] = useState<string[]>([])
  const [midiBusy, setMidiBusy] = useState(false)
  const [midiOutPorts, setMidiOutPorts] = useState<string[]>([])
  const [midiOutOpen, setMidiOutOpen] = useState<string[]>([])
  const [midiClockEnabled, setMidiClockEnabled] = useState(false)
  const [midiOutBusy, setMidiOutBusy] = useState(false)
  const [midiSyncEnabled, setMidiSyncEnabled] = useState(false)
  const [midiSyncTicksSeen, setMidiSyncTicksSeen] = useState(false)
  const [midiSyncBpm, setMidiSyncBpm] = useState<number | null>(null)
  const [midiMtcEnabled, setMidiMtcEnabled] = useState(false)
  const [midiMtcFps, setMidiMtcFps] = useState(30)
  const [directMonitoring, setDirectMonitoring] = useState(false)

  useEffect(() => {
    Promise.all([
      invoke<AudioDevice[]>('get_audio_devices'),
      invoke<AudioDevice[]>('get_audio_input_devices'),
      invoke<AudioConfig>('get_audio_config'),
      invoke<AudioInputConfig>('get_audio_input_config'),
      invoke<WasapiExclusiveStatus>('get_wasapi_exclusive'),
      invoke<string[]>('list_midi_inputs'),
      invoke<{ open_ports: string[] }>('get_midi_activity'),
      invoke<string[]>('list_midi_outputs'),
      invoke<{ enabled: boolean; open_ports: string[] }>('get_midi_clock_status'),
      invoke<{ enabled: boolean; ticks_seen: boolean; last_bpm: number | null }>('get_midi_clock_sync_status'),
      invoke<{ enabled: boolean; fps: number }>('get_midi_mtc_status'),
      invoke<boolean>('get_direct_monitoring'),
    ]).then(([devs, inputs, cfg, inCfg, excl, ports, activity, outPorts, clockStatus, syncStatus, mtcStatus, directMon]) => {
      setDevices(devs)
      setInputDevices(inputs)
      setConfig(cfg)
      setInputConfig(inCfg)
      setSelectedDevice(cfg.device)
      setSelectedInput(inCfg.device)
      setSelectedInputChannels(inCfg.channels)
      setSelectedRate(cfg.sample_rate)
      setSelectedBuffer(cfg.buffer_size)
      setExclusive(excl)
      setMidiPorts(ports)
      setMidiOpen(activity.open_ports)
      setMidiOutPorts(outPorts)
      setMidiOutOpen(clockStatus.open_ports)
      setMidiClockEnabled(clockStatus.enabled)
      setMidiSyncEnabled(syncStatus.enabled)
      setMidiSyncTicksSeen(syncStatus.ticks_seen)
      setMidiSyncBpm(syncStatus.last_bpm)
      setMidiMtcEnabled(mtcStatus.enabled)
      setMidiMtcFps(mtcStatus.fps)
      setDirectMonitoring(directMon)
    })
  }, [])

  // Poll clock-sync status so the BPM readout and ticks-seen indicator
  // update while the panel is open.
  useEffect(() => {
    const id = setInterval(async () => {
      try {
        const s = await invoke<{ enabled: boolean; ticks_seen: boolean; last_bpm: number | null }>(
          'get_midi_clock_sync_status'
        )
        setMidiSyncEnabled(s.enabled)
        setMidiSyncTicksSeen(s.ticks_seen)
        setMidiSyncBpm(s.last_bpm)
      } catch { /* ignore */ }
    }, 500)
    return () => clearInterval(id)
  }, [])

  const toggleMidiSync = async () => {
    const next = !midiSyncEnabled
    await invoke('set_midi_clock_sync_enabled', { enabled: next })
    setMidiSyncEnabled(next)
  }

  const toggleMidiOutPort = async (port: string) => {
    setError(null)
    setMidiOutBusy(true)
    try {
      if (midiOutOpen.includes(port)) {
        await invoke('close_midi_output', { portName: port })
        setMidiOutOpen(list => list.filter(p => p !== port))
      } else {
        await invoke('open_midi_output', { portName: port })
        setMidiOutOpen(list => [...list, port])
      }
    } catch (e: any) {
      setError(String(e))
    }
    setMidiOutBusy(false)
  }

  const rescanMidiOut = async () => {
    try {
      const ports = await invoke<string[]>('list_midi_outputs')
      setMidiOutPorts(ports)
    } catch (e: any) {
      setError(String(e))
    }
  }

  const toggleMidiClock = async () => {
    const next = !midiClockEnabled
    await invoke('set_midi_clock_enabled', { enabled: next })
    setMidiClockEnabled(next)
  }

  const toggleMidiMtc = async () => {
    const next = !midiMtcEnabled
    await invoke('set_midi_mtc_enabled', { enabled: next })
    setMidiMtcEnabled(next)
  }

  const changeMidiMtcFps = async (fps: number) => {
    try {
      await invoke('set_midi_mtc_fps', { fps })
      setMidiMtcFps(fps)
    } catch (e: any) {
      setError(String(e))
    }
  }

  const toggleDirectMonitoring = async () => {
    const next = !directMonitoring
    try {
      await invoke('set_direct_monitoring', { enabled: next })
      setDirectMonitoring(next)
    } catch (e: any) {
      setError(String(e))
    }
  }

  const toggleMidiPort = async (port: string) => {
    setError(null)
    setMidiBusy(true)
    try {
      if (midiOpen.includes(port)) {
        await invoke('close_midi_input', { portName: port })
        setMidiOpen(list => list.filter(p => p !== port))
      } else {
        await invoke('open_midi_input', { portName: port })
        setMidiOpen(list => [...list, port])
      }
    } catch (e: any) {
      setError(String(e))
    }
    setMidiBusy(false)
  }

  const rescanMidi = async () => {
    try {
      const ports = await invoke<string[]>('list_midi_inputs')
      setMidiPorts(ports)
    } catch (e: any) {
      setError(String(e))
    }
  }

  // Poll the input meter at ~30 Hz while monitoring is on. Decay the bar
  // smoothly on the UI side so brief silences don't make it flicker.
  useEffect(() => {
    if (!monitorOn) {
      peakDecayRef.current = { l: 0, r: 0 }
      setMonitor(null)
      return
    }
    let cancelled = false
    const tick = async () => {
      if (cancelled) return
      try {
        const snap = await invoke<InputMeterSnapshot>('get_input_meter')
        if (cancelled) return
        const decay = 0.85
        peakDecayRef.current = {
          l: Math.max(snap.peak_l, peakDecayRef.current.l * decay),
          r: Math.max(snap.peak_r, peakDecayRef.current.r * decay),
        }
        setMonitor({
          ...snap,
          peak_l: peakDecayRef.current.l,
          peak_r: peakDecayRef.current.r,
        })
      } catch {
        /* engine may be mid-restart; retry next tick */
      }
    }
    const id = window.setInterval(tick, 33)
    return () => {
      cancelled = true
      window.clearInterval(id)
    }
  }, [monitorOn])

  // Stop monitoring when the dialog is closed.
  useEffect(() => {
    return () => {
      invoke('stop_input_monitoring').catch(() => {})
    }
  }, [])

  const toggleMonitor = async () => {
    setError(null)
    if (monitorOn) {
      try {
        await invoke('stop_input_monitoring')
      } catch (e: any) {
        setError(String(e))
      }
      setMonitorOn(false)
      return
    }
    // Apply the currently-pending input device before starting the stream so
    // the monitor reflects the user's visible choice, not the last-applied one.
    try {
      await invoke('set_audio_input_config', {
        device: selectedInput,
        channels: selectedInputChannels,
      })
      setInputConfig({ device: selectedInput, channels: selectedInputChannels })
      await invoke('start_input_monitoring')
      setMonitorOn(true)
    } catch (e: any) {
      setError(String(e))
    }
  }

  const toggleExclusive = async () => {
    const next = !exclusive.enabled
    setError(null)
    try {
      await invoke('set_wasapi_exclusive', { enabled: next })
      setExclusive(s => ({ ...s, enabled: next }))
    } catch (e: any) {
      setError(String(e))
    }
  }

  const currentDevice = devices.find(d =>
    selectedDevice ? d.name === selectedDevice : d.is_default
  )
  const availableRates = currentDevice?.sample_rates.length
    ? currentDevice.sample_rates
    : [44100, 48000, 96000]

  const hasChanges =
    selectedDevice !== config.device ||
    selectedRate !== config.sample_rate ||
    selectedBuffer !== config.buffer_size ||
    selectedInput !== inputConfig.device ||
    selectedInputChannels !== inputConfig.channels

  const apply = async () => {
    setApplying(true)
    setError(null)
    try {
      await invoke('set_audio_config', {
        device: selectedDevice,
        sampleRate: selectedRate,
        bufferSize: selectedBuffer,
      })
      await invoke('set_audio_input_config', {
        device: selectedInput,
        channels: selectedInputChannels,
      })
      setConfig({ device: selectedDevice, sample_rate: selectedRate, buffer_size: selectedBuffer })
      setInputConfig({ device: selectedInput, channels: selectedInputChannels })
    } catch (e: any) {
      setError(String(e))
    }
    setApplying(false)
  }

  const latencyMs = ((selectedBuffer / selectedRate) * 1000).toFixed(1)

  return (
    <div style={{
      position: 'fixed', inset: 0, zIndex: 90,
      display: 'flex', alignItems: 'center', justifyContent: 'center',
      background: 'rgba(0,0,0,0.6)', backdropFilter: 'blur(4px)',
    }} onClick={onClose}>
      <div
        onClick={e => e.stopPropagation()}
        style={{
          width: 480, background: 'rgba(12,12,16,0.98)',
          border: `1px solid ${hw.borderLight}`,
          borderRadius: hw.radius.lg,
          boxShadow: '0 20px 60px rgba(0,0,0,0.7)',
          overflow: 'hidden',
        }}
      >
        {/* Header */}
        <div style={{
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          padding: '12px 16px',
          background: 'rgba(255,255,255,0.03)',
          borderBottom: `1px solid ${hw.border}`,
        }}>
          <span style={{ fontSize: 13, fontWeight: 700, color: hw.accent, letterSpacing: 0.5 }}>
            AUDIO SETTINGS
          </span>
          <div
            onClick={onClose}
            style={{ width: 20, height: 20, display: 'flex', alignItems: 'center', justifyContent: 'center',
              borderRadius: 4, color: hw.textFaint, cursor: 'pointer' }}
            onMouseEnter={e => { e.currentTarget.style.background = 'rgba(255,255,255,0.08)'; e.currentTarget.style.color = hw.textPrimary }}
            onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = hw.textFaint }}
          >
            <svg width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
              <line x1="2" y1="2" x2="10" y2="10" /><line x1="10" y1="2" x2="2" y2="10" />
            </svg>
          </div>
        </div>

        {/* Body */}
        <div style={{ padding: '16px 16px 12px' }}>
          {/* Output Device */}
          <SettingRow label="Output Device">
            <Select
              value={selectedDevice ?? ''}
              onChange={v => {
                const dev = v === '' ? null : v
                setSelectedDevice(dev)
                // Auto-adjust sample rate if current isn't supported
                const d = devices.find(d => d.name === v)
                if (d && d.sample_rates.length > 0 && !d.sample_rates.includes(selectedRate)) {
                  setSelectedRate(d.sample_rates.includes(48000) ? 48000 : d.sample_rates[0])
                }
              }}
              options={[
                { value: '', label: `System Default${devices.find(d => d.is_default) ? ` (${devices.find(d => d.is_default)!.name})` : ''}` },
                ...devices.map(d => ({ value: d.name, label: d.name })),
              ]}
            />
          </SettingRow>

          {/* Sample Rate */}
          <SettingRow label="Sample Rate">
            <Select
              value={String(selectedRate)}
              onChange={v => setSelectedRate(Number(v))}
              options={availableRates.map(r => ({
                value: String(r),
                label: `${(r / 1000).toFixed(r % 1000 === 0 ? 0 : 1)} kHz`,
              }))}
            />
          </SettingRow>

          {/* Buffer Size */}
          <SettingRow label="Buffer Size">
            <Select
              value={String(selectedBuffer)}
              onChange={v => setSelectedBuffer(Number(v))}
              options={BUFFER_SIZES.map(b => ({
                value: String(b),
                label: `${b} samples`,
              }))}
            />
          </SettingRow>

          {/* Input Device */}
          <SettingRow label="Input Device">
            <Select
              value={selectedInput ?? ''}
              onChange={v => setSelectedInput(v === '' ? null : v)}
              options={[
                { value: '', label: `System Default${inputDevices.find(d => d.is_default) ? ` (${inputDevices.find(d => d.is_default)!.name})` : ''}` },
                ...inputDevices.map(d => ({ value: d.name, label: d.name })),
                ...(inputDevices.length === 0 ? [{ value: '', label: 'No input devices found' }] : []),
              ]}
            />
          </SettingRow>

          {/* Input Channels */}
          <SettingRow label="Input Channels">
            <Select
              value={String(selectedInputChannels)}
              onChange={v => setSelectedInputChannels(Number(v))}
              options={[
                { value: '1', label: 'Mono (1 channel, summed)' },
                { value: '2', label: 'Stereo (2 channels)' },
              ]}
            />
          </SettingRow>

          {/* Input Monitor / pre-record level meter */}
          <div style={{
            marginBottom: 10, padding: '8px 12px',
            background: hw.bgPanel, borderRadius: hw.radius.sm,
            border: `1px solid ${hw.borderDark}`,
          }}>
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: monitorOn ? 6 : 0 }}>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                <span style={{ fontSize: 11, color: hw.textMuted }}>Input Monitor</span>
                <span style={{ fontSize: 10, color: hw.textFaint }}>
                  {monitorOn && monitor?.running
                    ? `Live — ${monitor.sample_rate} Hz, ${monitor.buffer_size} samples`
                    : 'Preview input signal level before recording'}
                </span>
              </div>
              <button
                onClick={toggleMonitor}
                style={{
                  padding: '4px 12px', fontSize: 11, fontWeight: 600,
                  borderRadius: hw.radius.sm, border: 'none',
                  cursor: 'pointer',
                  background: monitorOn ? hw.accent : 'rgba(255,255,255,0.08)',
                  color: monitorOn ? '#fff' : hw.textSecondary,
                  fontFamily: 'inherit',
                }}
              >
                {monitorOn ? 'Stop' : 'Start'}
              </button>
            </div>
            {monitorOn && (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
                <MeterBar label="L" peak={monitor?.peak_l ?? 0} />
                {selectedInputChannels === 2 && (
                  <MeterBar label="R" peak={monitor?.peak_r ?? 0} />
                )}
              </div>
            )}
            <div style={{
              marginTop: 8,
              display: 'flex', alignItems: 'center', justifyContent: 'space-between',
              padding: '6px 8px',
              background: 'rgba(0,0,0,0.25)',
              borderRadius: hw.radius.sm,
              border: `1px solid ${directMonitoring ? hw.accent : hw.borderDark}`,
            }}>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                <span style={{ fontSize: 11, color: hw.textSecondary }}>Direct monitoring</span>
                <span style={{ fontSize: 9, color: hw.textFaint }}>
                  {directMonitoring
                    ? 'Live input routes straight to master — track FX bypassed.'
                    : 'Software monitor: live input passes through the track FX chain.'}
                </span>
              </div>
              <button
                onClick={toggleDirectMonitoring}
                style={{
                  padding: '2px 10px', fontSize: 10, fontWeight: 600,
                  borderRadius: hw.radius.sm, border: 'none',
                  cursor: 'pointer',
                  background: directMonitoring ? hw.accent : 'rgba(255,255,255,0.08)',
                  color: directMonitoring ? '#fff' : hw.textSecondary,
                  fontFamily: 'inherit',
                }}
              >
                {directMonitoring ? 'On' : 'Off'}
              </button>
            </div>
          </div>

          {/* MIDI Inputs */}
          <div style={{
            marginBottom: 10, padding: '8px 12px',
            background: hw.bgPanel, borderRadius: hw.radius.sm,
            border: `1px solid ${hw.borderDark}`,
          }}>
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: midiPorts.length > 0 ? 6 : 0 }}>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                <span style={{ fontSize: 11, color: hw.textMuted }}>MIDI Inputs</span>
                <span style={{ fontSize: 10, color: hw.textFaint }}>
                  {midiPorts.length === 0
                    ? 'No MIDI input ports detected'
                    : `${midiOpen.length} of ${midiPorts.length} open`}
                </span>
              </div>
              <button
                onClick={rescanMidi}
                style={{
                  padding: '4px 10px', fontSize: 11, fontWeight: 600,
                  borderRadius: hw.radius.sm, border: 'none',
                  cursor: 'pointer',
                  background: 'rgba(255,255,255,0.08)',
                  color: hw.textSecondary,
                  fontFamily: 'inherit',
                }}
              >
                Rescan
              </button>
            </div>
            {midiPorts.length > 0 && (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
                {midiPorts.map(port => {
                  const open = midiOpen.includes(port)
                  return (
                    <div key={port} style={{
                      display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                      padding: '4px 8px',
                      background: 'rgba(0,0,0,0.25)',
                      borderRadius: hw.radius.sm,
                      border: `1px solid ${open ? hw.accent : hw.borderDark}`,
                    }}>
                      <span style={{ fontSize: 11, color: hw.textSecondary, fontFamily: 'inherit' }}>
                        {port}
                      </span>
                      <button
                        disabled={midiBusy}
                        onClick={() => toggleMidiPort(port)}
                        style={{
                          padding: '2px 10px', fontSize: 10, fontWeight: 600,
                          borderRadius: hw.radius.sm, border: 'none',
                          cursor: midiBusy ? 'default' : 'pointer',
                          background: open ? hw.accent : 'rgba(255,255,255,0.08)',
                          color: open ? '#fff' : hw.textSecondary,
                          fontFamily: 'inherit',
                        }}
                      >
                        {open ? 'Connected' : 'Connect'}
                      </button>
                    </div>
                  )
                })}
              </div>
            )}
            <div style={{
              marginTop: midiPorts.length > 0 ? 8 : 6,
              display: 'flex', alignItems: 'center', justifyContent: 'space-between',
              padding: '6px 8px',
              background: 'rgba(0,0,0,0.25)',
              borderRadius: hw.radius.sm,
              border: `1px solid ${midiSyncEnabled ? hw.accent : hw.borderDark}`,
              opacity: midiOpen.length === 0 ? 0.6 : 1,
            }}>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                <span style={{ fontSize: 11, color: hw.textSecondary }}>Sync to external clock</span>
                <span style={{ fontSize: 9, color: hw.textFaint, fontFamily: "'Consolas', monospace" }}>
                  {midiSyncTicksSeen
                    ? (midiSyncBpm != null
                      ? `Master: ${midiSyncBpm.toFixed(2)} BPM`
                      : 'Master detected · waiting for stable tempo')
                    : 'No clock ticks received yet'}
                </span>
              </div>
              <button
                onClick={toggleMidiSync}
                disabled={midiOpen.length === 0}
                style={{
                  padding: '2px 10px', fontSize: 10, fontWeight: 600,
                  borderRadius: hw.radius.sm, border: 'none',
                  cursor: midiOpen.length === 0 ? 'default' : 'pointer',
                  background: midiSyncEnabled ? hw.accent : 'rgba(255,255,255,0.08)',
                  color: midiSyncEnabled ? '#fff' : hw.textSecondary,
                  fontFamily: 'inherit',
                }}
              >
                {midiSyncEnabled ? 'On' : 'Off'}
              </button>
            </div>
          </div>

          {/* MIDI Outputs — clock send to external hardware */}
          <div style={{
            marginBottom: 10, padding: '8px 12px',
            background: hw.bgPanel, borderRadius: hw.radius.sm,
            border: `1px solid ${hw.borderDark}`,
          }}>
            <div style={{
              display: 'flex', alignItems: 'center', justifyContent: 'space-between',
              marginBottom: midiOutPorts.length > 0 ? 6 : 0,
            }}>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                <span style={{ fontSize: 11, color: hw.textMuted }}>MIDI Outputs</span>
                <span style={{ fontSize: 10, color: hw.textFaint }}>
                  {midiOutPorts.length === 0
                    ? 'No MIDI output ports detected'
                    : `${midiOutOpen.length} of ${midiOutPorts.length} open`}
                </span>
              </div>
              <button
                onClick={rescanMidiOut}
                style={{
                  padding: '4px 10px', fontSize: 11, fontWeight: 600,
                  borderRadius: hw.radius.sm, border: 'none',
                  cursor: 'pointer',
                  background: 'rgba(255,255,255,0.08)',
                  color: hw.textSecondary,
                  fontFamily: 'inherit',
                }}
              >
                Rescan
              </button>
            </div>
            {midiOutPorts.length > 0 && (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
                {midiOutPorts.map(port => {
                  const open = midiOutOpen.includes(port)
                  return (
                    <div key={port} style={{
                      display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                      padding: '4px 8px',
                      background: 'rgba(0,0,0,0.25)',
                      borderRadius: hw.radius.sm,
                      border: `1px solid ${open ? hw.accent : hw.borderDark}`,
                    }}>
                      <span style={{ fontSize: 11, color: hw.textSecondary, fontFamily: 'inherit' }}>
                        {port}
                      </span>
                      <button
                        disabled={midiOutBusy}
                        onClick={() => toggleMidiOutPort(port)}
                        style={{
                          padding: '2px 10px', fontSize: 10, fontWeight: 600,
                          borderRadius: hw.radius.sm, border: 'none',
                          cursor: midiOutBusy ? 'default' : 'pointer',
                          background: open ? hw.accent : 'rgba(255,255,255,0.08)',
                          color: open ? '#fff' : hw.textSecondary,
                          fontFamily: 'inherit',
                        }}
                      >
                        {open ? 'Connected' : 'Connect'}
                      </button>
                    </div>
                  )
                })}
              </div>
            )}
            <div style={{
              marginTop: midiOutPorts.length > 0 ? 8 : 6,
              display: 'flex', alignItems: 'center', justifyContent: 'space-between',
              padding: '6px 8px',
              background: 'rgba(0,0,0,0.25)',
              borderRadius: hw.radius.sm,
              border: `1px solid ${midiClockEnabled ? hw.accent : hw.borderDark}`,
              opacity: midiOutOpen.length === 0 ? 0.6 : 1,
            }}>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                <span style={{ fontSize: 11, color: hw.textSecondary }}>Send MIDI clock</span>
                <span style={{ fontSize: 9, color: hw.textFaint }}>
                  Broadcasts 24 PPQN clock + Start/Stop to every open output.
                </span>
              </div>
              <button
                onClick={toggleMidiClock}
                disabled={midiOutOpen.length === 0}
                style={{
                  padding: '2px 10px', fontSize: 10, fontWeight: 600,
                  borderRadius: hw.radius.sm, border: 'none',
                  cursor: midiOutOpen.length === 0 ? 'default' : 'pointer',
                  background: midiClockEnabled ? hw.accent : 'rgba(255,255,255,0.08)',
                  color: midiClockEnabled ? '#fff' : hw.textSecondary,
                  fontFamily: 'inherit',
                }}
              >
                {midiClockEnabled ? 'On' : 'Off'}
              </button>
            </div>
            <div style={{
              marginTop: 6,
              display: 'flex', alignItems: 'center', justifyContent: 'space-between',
              padding: '6px 8px',
              background: 'rgba(0,0,0,0.25)',
              borderRadius: hw.radius.sm,
              border: `1px solid ${midiMtcEnabled ? hw.accent : hw.borderDark}`,
              opacity: midiOutOpen.length === 0 ? 0.6 : 1,
            }}>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                <span style={{ fontSize: 11, color: hw.textSecondary }}>Send MIDI timecode</span>
                <span style={{ fontSize: 9, color: hw.textFaint }}>
                  Broadcasts SMPTE timecode as MTC Quarter Frames while playing.
                </span>
              </div>
              <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                <select
                  value={midiMtcFps}
                  onChange={(e) => changeMidiMtcFps(Number(e.target.value))}
                  disabled={midiOutOpen.length === 0}
                  style={{
                    fontSize: 10,
                    background: 'rgba(255,255,255,0.05)',
                    color: hw.textSecondary,
                    border: `1px solid ${hw.borderDark}`,
                    borderRadius: hw.radius.sm,
                    padding: '2px 4px',
                    fontFamily: 'inherit',
                  }}
                >
                  <option value={24}>24 fps</option>
                  <option value={25}>25 fps</option>
                  <option value={30}>30 fps</option>
                </select>
                <button
                  onClick={toggleMidiMtc}
                  disabled={midiOutOpen.length === 0}
                  style={{
                    padding: '2px 10px', fontSize: 10, fontWeight: 600,
                    borderRadius: hw.radius.sm, border: 'none',
                    cursor: midiOutOpen.length === 0 ? 'default' : 'pointer',
                    background: midiMtcEnabled ? hw.accent : 'rgba(255,255,255,0.08)',
                    color: midiMtcEnabled ? '#fff' : hw.textSecondary,
                    fontFamily: 'inherit',
                  }}
                >
                  {midiMtcEnabled ? 'On' : 'Off'}
                </button>
              </div>
            </div>
          </div>

          {/* Latency readout */}
          <div style={{
            display: 'flex', justifyContent: 'space-between', alignItems: 'center',
            marginTop: 12, padding: '8px 12px',
            background: hw.bgPanel, borderRadius: hw.radius.sm,
            border: `1px solid ${hw.borderDark}`,
          }}>
            <span style={{ fontSize: 11, color: hw.textMuted }}>Output Latency</span>
            <span style={{ fontSize: 12, fontWeight: 600, color: hw.textPrimary, fontFamily: "'Consolas', monospace" }}>
              {latencyMs} ms
            </span>
          </div>

          {/* WASAPI exclusive-mode toggle (Windows only) */}
          <div style={{
            display: 'flex', justifyContent: 'space-between', alignItems: 'center',
            marginTop: 8, padding: '8px 12px',
            background: hw.bgPanel, borderRadius: hw.radius.sm,
            border: `1px solid ${hw.borderDark}`,
            opacity: exclusive.available ? 1 : 0.5,
          }}>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
              <span style={{ fontSize: 11, color: hw.textMuted }}>WASAPI Exclusive Mode</span>
              <span style={{ fontSize: 10, color: hw.textFaint }}>
                {exclusive.available
                  ? 'Lowest latency, locks the output device to the DAW'
                  : 'Only available on Windows with WASAPI host'}
              </span>
            </div>
            <button
              disabled={!exclusive.available}
              onClick={toggleExclusive}
              style={{
                width: 38, height: 20,
                borderRadius: 10, border: 'none',
                background: exclusive.enabled ? hw.accent : 'rgba(255,255,255,0.08)',
                cursor: exclusive.available ? 'pointer' : 'default',
                position: 'relative', transition: 'background 150ms ease',
              }}
            >
              <span style={{
                position: 'absolute', top: 2, left: exclusive.enabled ? 20 : 2,
                width: 16, height: 16, borderRadius: '50%',
                background: '#fff', transition: 'left 150ms ease',
              }} />
            </button>
          </div>

          {error && (
            <div style={{ marginTop: 8, fontSize: 11, color: hw.red, padding: '6px 10px', background: hw.redDim, borderRadius: hw.radius.sm }}>
              {error}
            </div>
          )}
        </div>

        {/* Footer */}
        <div style={{
          display: 'flex', justifyContent: 'flex-end', gap: 8,
          padding: '10px 16px',
          borderTop: `1px solid ${hw.border}`,
          background: 'rgba(255,255,255,0.02)',
        }}>
          <Btn label="Close" onClick={onClose} />
          <Btn label={applying ? 'Applying...' : 'Apply'} primary disabled={!hasChanges || applying} onClick={apply} />
        </div>
      </div>
    </div>
  )
}

function SettingRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 10 }}>
      <span style={{ fontSize: 12, color: hw.textSecondary }}>{label}</span>
      {children}
    </div>
  )
}

function Select({ value, onChange, options }: {
  value: string
  onChange: (v: string) => void
  options: { value: string; label: string }[]
}) {
  return (
    <select
      value={value}
      onChange={e => onChange(e.target.value)}
      style={{
        width: 260, padding: '5px 8px',
        background: hw.bgInput, color: hw.textPrimary,
        border: `1px solid ${hw.border}`, borderRadius: hw.radius.sm,
        fontSize: 12, fontFamily: 'inherit', outline: 'none',
        cursor: 'pointer',
      }}
    >
      {options.map(o => (
        <option key={o.value} value={o.value} style={{ background: '#0c0c10' }}>{o.label}</option>
      ))}
    </select>
  )
}

function MeterBar({ label, peak }: { label: string; peak: number }) {
  // Convert linear peak to dB for display scaling. Map -60..+6 dB to 0..100%.
  const db = peak > 0 ? 20 * Math.log10(peak) : -100
  const pct = Math.max(0, Math.min(100, ((db + 60) / 66) * 100))
  const clipping = peak >= 0.999
  const color = clipping ? hw.red : db > -6 ? '#facc15' : hw.accent
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
      <span style={{ fontSize: 9, color: hw.textFaint, width: 8 }}>{label}</span>
      <div style={{
        flex: 1, height: 6, background: 'rgba(0,0,0,0.4)',
        borderRadius: 2, overflow: 'hidden', border: `1px solid ${hw.borderDark}`,
      }}>
        <div style={{
          width: `${pct}%`, height: '100%',
          background: color,
          transition: 'width 50ms linear',
        }} />
      </div>
      <span style={{
        fontSize: 9, color: hw.textFaint,
        width: 36, textAlign: 'right',
        fontFamily: "'Consolas', monospace",
      }}>
        {db <= -60 ? '-∞' : `${db.toFixed(1)}`}
      </span>
    </div>
  )
}

function Btn({ label, primary, disabled, onClick }: {
  label: string; primary?: boolean; disabled?: boolean; onClick: () => void
}) {
  return (
    <button
      disabled={disabled}
      onClick={onClick}
      style={{
        padding: '5px 16px', fontSize: 12, fontWeight: 600,
        borderRadius: hw.radius.sm, border: 'none', cursor: disabled ? 'default' : 'pointer',
        background: primary ? (disabled ? 'rgba(220,38,38,0.3)' : hw.accent) : 'rgba(255,255,255,0.06)',
        color: primary ? (disabled ? 'rgba(255,255,255,0.4)' : '#fff') : hw.textSecondary,
        fontFamily: 'inherit',
      }}
    >
      {label}
    </button>
  )
}
