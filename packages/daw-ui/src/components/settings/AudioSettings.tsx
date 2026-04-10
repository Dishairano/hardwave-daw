import { useState, useEffect } from 'react'
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

interface AudioSettingsProps {
  onClose: () => void
}

const BUFFER_SIZES = [64, 128, 256, 512, 1024, 2048, 4096]

export function AudioSettings({ onClose }: AudioSettingsProps) {
  const [devices, setDevices] = useState<AudioDevice[]>([])
  const [config, setConfig] = useState<AudioConfig>({ device: null, sample_rate: 48000, buffer_size: 512 })
  const [selectedDevice, setSelectedDevice] = useState<string | null>(null)
  const [selectedRate, setSelectedRate] = useState(48000)
  const [selectedBuffer, setSelectedBuffer] = useState(512)
  const [applying, setApplying] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    Promise.all([
      invoke<AudioDevice[]>('get_audio_devices'),
      invoke<AudioConfig>('get_audio_config'),
    ]).then(([devs, cfg]) => {
      setDevices(devs)
      setConfig(cfg)
      setSelectedDevice(cfg.device)
      setSelectedRate(cfg.sample_rate)
      setSelectedBuffer(cfg.buffer_size)
    })
  }, [])

  const currentDevice = devices.find(d =>
    selectedDevice ? d.name === selectedDevice : d.is_default
  )
  const availableRates = currentDevice?.sample_rates.length
    ? currentDevice.sample_rates
    : [44100, 48000, 96000]

  const hasChanges =
    selectedDevice !== config.device ||
    selectedRate !== config.sample_rate ||
    selectedBuffer !== config.buffer_size

  const apply = async () => {
    setApplying(true)
    setError(null)
    try {
      await invoke('set_audio_config', {
        device: selectedDevice,
        sampleRate: selectedRate,
        bufferSize: selectedBuffer,
      })
      setConfig({ device: selectedDevice, sample_rate: selectedRate, buffer_size: selectedBuffer })
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
