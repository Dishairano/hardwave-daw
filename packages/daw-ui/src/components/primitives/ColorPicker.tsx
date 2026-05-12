import { useEffect, useRef, useState } from 'react'
import { useColorPickerStore } from '../../stores/colorPickerStore'
import './ColorPicker.css'

/**
 * Global color picker popover.
 *
 * Mounted once at the App level. Reads its open/close state + `onPick`
 * callback from `useColorPickerStore`. Renders nothing when closed,
 * floats above all panels when open.
 *
 * Implements the FL Studio "Color Selector" feature set from the
 * manual page "The User Interface":
 *
 *  - Preset palette row (track/clip colour defaults)
 *  - Recent-colours row (localStorage-persisted)
 *  - HSL square (S × L) + hue slider — drag picks the custom colour
 *  - Hex input (#RRGGBB) with live commit
 *  - Default button (clears override / resets)
 *  - Accept button — applies the custom hex
 *  - Esc / click-outside closes
 *
 * Skipped vs FL: "Lock to safe colors" + random-safe dice + magnet
 * toggle — Hardwave's default palette already only contains
 * safe-luminance shades, so the magnet is implicit.
 */

const PRESET_COLORS = [
  '#DC2626', '#EF4444', '#F97316', '#F59E0B',
  '#EAB308', '#84CC16', '#22C55E', '#10B981',
  '#06B6D4', '#3B82F6', '#6366F1', '#8B5CF6',
  '#A855F7', '#D946EF', '#EC4899', '#F43F5E',
]

function hexFromHsl(h: number, s: number, l: number): string {
  // h in [0..360), s+l in [0..100]
  const sat = s / 100
  const lit = l / 100
  const c = (1 - Math.abs(2 * lit - 1)) * sat
  const hp = h / 60
  const x = c * (1 - Math.abs((hp % 2) - 1))
  let r = 0, g = 0, b = 0
  if (0 <= hp && hp < 1) [r, g, b] = [c, x, 0]
  else if (hp < 2) [r, g, b] = [x, c, 0]
  else if (hp < 3) [r, g, b] = [0, c, x]
  else if (hp < 4) [r, g, b] = [0, x, c]
  else if (hp < 5) [r, g, b] = [x, 0, c]
  else if (hp < 6) [r, g, b] = [c, 0, x]
  const m = lit - c / 2
  const to = (v: number) => Math.round((v + m) * 255).toString(16).padStart(2, '0')
  return `#${to(r)}${to(g)}${to(b)}`.toUpperCase()
}

function hslFromHex(hex: string): { h: number; s: number; l: number } {
  const m = /^#?([a-f0-9]{6})$/i.exec(hex.trim())
  if (!m) return { h: 0, s: 0, l: 50 }
  const n = parseInt(m[1], 16)
  const r = ((n >> 16) & 0xff) / 255
  const g = ((n >> 8) & 0xff) / 255
  const b = (n & 0xff) / 255
  const max = Math.max(r, g, b)
  const min = Math.min(r, g, b)
  const l = (max + min) / 2
  let h = 0
  let s = 0
  if (max !== min) {
    const d = max - min
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min)
    switch (max) {
      case r: h = ((g - b) / d + (g < b ? 6 : 0)) * 60; break
      case g: h = ((b - r) / d + 2) * 60; break
      case b: h = ((r - g) / d + 4) * 60; break
    }
  }
  return { h, s: s * 100, l: l * 100 }
}

function isValidHex(s: string): boolean {
  return /^#?[a-f0-9]{6}$/i.test(s.trim())
}

function normalizeHex(s: string): string {
  return ('#' + s.trim().replace(/^#/, '')).toUpperCase()
}

export function ColorPicker() {
  const args = useColorPickerStore((s) => s.args)
  const recent = useColorPickerStore((s) => s.recent)
  const close = useColorPickerStore((s) => s.close)
  const markUsed = useColorPickerStore((s) => s.markUsed)

  const ref = useRef<HTMLDivElement | null>(null)
  const [hue, setHue] = useState(0)
  const [sat, setSat] = useState(70)
  const [lit, setLit] = useState(50)
  const [hexInput, setHexInput] = useState('')

  // Re-seed picker state whenever `args.current` changes.
  useEffect(() => {
    if (!args) return
    const seed = args.current || PRESET_COLORS[0]
    const parsed = hslFromHex(seed)
    setHue(parsed.h)
    setSat(parsed.s)
    setLit(parsed.l)
    setHexInput(normalizeHex(seed))
  }, [args])

  // Outside-click + Escape closes.
  useEffect(() => {
    if (!args) return
    const onPointerDown = (e: PointerEvent) => {
      if (!ref.current) return
      if (!ref.current.contains(e.target as Node)) close()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        close()
      }
    }
    // Defer pointerdown attach so the click that opened us doesn't
    // immediately close us via the same event.
    const t = window.setTimeout(() => {
      window.addEventListener('pointerdown', onPointerDown)
    }, 0)
    window.addEventListener('keydown', onKey)
    return () => {
      window.clearTimeout(t)
      window.removeEventListener('pointerdown', onPointerDown)
      window.removeEventListener('keydown', onKey)
    }
  }, [args, close])

  if (!args) return null

  // Position popover near the anchor — prefer below, clamp inside the
  // viewport. Width is fixed at 240 px.
  const POPOVER_W = 240
  const POPOVER_H = 320
  const vw = window.innerWidth
  const vh = window.innerHeight
  let left = args.anchor.left
  let top = args.anchor.bottom + 6
  if (left + POPOVER_W > vw - 8) left = vw - POPOVER_W - 8
  if (top + POPOVER_H > vh - 8) top = args.anchor.top - POPOVER_H - 6
  if (top < 8) top = 8
  if (left < 8) left = 8

  const liveHex = hexFromHsl(hue, sat, lit)

  const applyColor = (c: string) => {
    args.onPick(c)
    markUsed(c)
    close()
  }

  const handleHexInputCommit = () => {
    if (isValidHex(hexInput)) applyColor(normalizeHex(hexInput))
  }

  return (
    <div
      ref={ref}
      className="hw-color-picker"
      role="dialog"
      aria-label={args.title ?? 'Pick a color'}
      style={{ left, top }}
      onPointerDown={(e) => e.stopPropagation()}
    >
      {args.title && <div className="hw-cp-title">{args.title}</div>}

      {/* Preset palette */}
      <div className="hw-cp-section-label">Presets</div>
      <div className="hw-cp-grid">
        {PRESET_COLORS.map((c) => (
          <button
            key={c}
            type="button"
            title={c}
            onClick={() => applyColor(c)}
            className={
              'hw-cp-swatch' +
              (args.current && args.current.toLowerCase() === c.toLowerCase() ? ' active' : '')
            }
            style={{ background: c }}
          />
        ))}
      </div>

      {/* Recent */}
      {recent.length > 0 && (
        <>
          <div className="hw-cp-section-label">Recent</div>
          <div className="hw-cp-grid">
            {recent.slice(0, 12).map((c) => (
              <button
                key={c}
                type="button"
                title={c}
                onClick={() => applyColor(c)}
                className="hw-cp-swatch"
                style={{ background: c }}
              />
            ))}
          </div>
        </>
      )}

      {/* HSL custom */}
      <div className="hw-cp-section-label">Custom</div>
      <div
        className="hw-cp-sl"
        style={
          {
            ['--hue' as string]: `${hue}`,
          } as React.CSSProperties
        }
        onPointerDown={(e) => {
          e.preventDefault()
          const el = e.currentTarget
          el.setPointerCapture(e.pointerId)
          const update = (ev: PointerEvent | React.PointerEvent) => {
            const rect = el.getBoundingClientRect()
            const x = Math.max(0, Math.min(1, (ev.clientX - rect.left) / rect.width))
            const y = Math.max(0, Math.min(1, (ev.clientY - rect.top) / rect.height))
            setSat(x * 100)
            setLit((1 - y) * 100)
          }
          update(e)
          const onMove = (ev: PointerEvent) => update(ev)
          const onUp = () => {
            el.removeEventListener('pointermove', onMove)
            el.removeEventListener('pointerup', onUp)
          }
          el.addEventListener('pointermove', onMove)
          el.addEventListener('pointerup', onUp)
        }}
      >
        <div className="hw-cp-sl-cursor" style={{ left: `${sat}%`, top: `${100 - lit}%` }} />
      </div>
      <input
        type="range"
        min={0}
        max={359}
        step={1}
        value={Math.round(hue)}
        onChange={(e) => setHue(parseInt(e.target.value, 10))}
        className="hw-cp-hue"
      />

      {/* Hex + actions */}
      <div className="hw-cp-row">
        <span
          className="hw-cp-preview"
          style={{ background: liveHex }}
          title={liveHex}
        />
        <input
          type="text"
          className="hw-cp-hex"
          value={hexInput}
          onChange={(e) => setHexInput(e.target.value)}
          onBlur={() => isValidHex(hexInput) && setHexInput(normalizeHex(hexInput))}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              handleHexInputCommit()
              e.preventDefault()
            }
          }}
        />
        <button
          type="button"
          className="hw-cp-apply"
          onClick={() => applyColor(liveHex)}
          title="Apply custom color"
        >
          Apply
        </button>
      </div>
      <div className="hw-cp-row hw-cp-row-foot">
        {args.onClear && (
          <button
            type="button"
            className="hw-cp-clear"
            onClick={() => {
              args.onClear?.()
              close()
            }}
            title="Reset to default colour"
          >
            Default
          </button>
        )}
        <span style={{ flex: 1 }} />
        <button
          type="button"
          className="hw-cp-cancel"
          onClick={() => close()}
        >
          Close
        </button>
      </div>
    </div>
  )
}
