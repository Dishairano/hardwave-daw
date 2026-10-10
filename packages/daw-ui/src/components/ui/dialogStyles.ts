import type { CSSProperties } from 'react'

/**
 * Inline styles shared by the dialogs that sit in DialogFrame. Each dialog
 * used to carry its own copy of these, in 9 and 10 px.
 */
const SANS = "Inter, 'Segoe UI', system-ui, sans-serif"

/** A button; `active` makes it the red main action. */
export function btn(active: boolean = false): CSSProperties {
  return {
    height: 30, padding: '0 12px', borderRadius: 8,
    fontFamily: SANS, fontSize: 12, fontWeight: 600, cursor: 'pointer',
    background: active ? '#dc2626' : '#1e1e23',
    border: `1px solid ${active ? '#dc2626' : 'rgba(255,255,255,0.1)'}`,
    color: active ? '#fff' : '#ececf0',
  }
}

export function sel(): CSSProperties {
  return {
    height: 30, padding: '0 8px', borderRadius: 8, maxWidth: 200,
    background: '#0a0a0b', border: '1px solid rgba(255,255,255,0.12)',
    color: '#ececf0', fontFamily: SANS, fontSize: 12.5,
  }
}

export function th(): CSSProperties {
  return { textAlign: 'left', padding: '10px 10px 8px', fontSize: 11.5, fontWeight: 600, color: '#8a8a94' }
}

export function td(): CSSProperties {
  return { padding: '8px 10px', fontVariantNumeric: 'tabular-nums' }
}
