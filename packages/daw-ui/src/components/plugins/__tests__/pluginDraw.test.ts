import { describe, expect, it } from 'vitest'
import { ParamView, biquadDb, valueForNumber, type PluginParam } from '../pluginDraw'
import { LAYOUTS } from '../pluginLayouts'
import table from '../../../dev/builtinParams.json'

describe('the plug-ins biquad, as drawn', () => {
  it('a bell reaches its gain at its frequency', () => {
    expect(biquadDb('peak', 1000, 1, 6, 1000)).toBeCloseTo(6, 1)
    expect(Math.abs(biquadDb('peak', 1000, 1, 6, 50))).toBeLessThan(0.2)
  })
  it('a low pass at Q 0.707 is 3 dB down at its cutoff', () => {
    expect(biquadDb('lowpass', 1000, Math.SQRT1_2, 0, 1000)).toBeCloseTo(-3, 0)
  })
  it('a low shelf lifts the lows and leaves the highs', () => {
    expect(biquadDb('lowshelf', 100, 0.7, 6, 20)).toBeGreaterThan(5)
    expect(Math.abs(biquadDb('lowshelf', 100, 0.7, 6, 10000))).toBeLessThan(0.2)
  })
})

describe('setting a parameter by what it reads', () => {
  const params = (table as Record<string, { params: PluginParam[] }>)['hardwave.native.filter'].params
  const view = new ParamView(params, {}, {})
  it('finds the knob position for a frequency on a 0..1 parameter', () => {
    const cutoff = view.par('Cutoff')!
    const v = valueForNumber(view, cutoff, 2000)
    const shown = new ParamView(params, { [cutoff.id]: v }, {}).num('Cutoff')
    expect(Math.abs(shown - 2000) / 2000).toBeLessThan(0.08)
  })
})

describe('every layout names real parameters', () => {
  it('each knob, choice and switch exists in its plug-in', () => {
    const missing: string[] = []
    for (const [key, l] of Object.entries(LAYOUTS)) {
      const names = new Set((table as Record<string, { params: PluginParam[] }>)[`hardwave.native.${key}`].params.map((p) => p.name))
      const check = (n: string) => { if (!names.has(n)) missing.push(`${key}: ${n}`) }
      const panels = [...(l.panels ?? []).flat(), ...(l.top ? [l.top] : []), ...(l.bottom ? [l.bottom] : [])]
      for (const p of panels) {
        if (p.on) check(p.on)
        for (const rows of p.s) for (const r of rows) {
          if (r[0] === 'k') (r.slice(1) as string[]).forEach(check)
          if (r[0] === 'K' || r[0] === 'p' || r[0] === 'w') check(r[1] as string)
          if (r[0] === 'd' && (r[1] as string).startsWith('wave:')) check((r[1] as string).slice(5))
          if (r[0] === 'r') (r.slice(1) as string[]).filter((x) => x.startsWith('param:')).forEach((x) => check(x.slice(6)))
        }
      }
      if (l.mix) check(l.mix)
      if (l.hp) check(l.hp)
    }
    expect(missing).toEqual([])
  })
  it('every built-in has a layout', () => {
    const keys = Object.keys(table).map((k) => k.replace('hardwave.native.', ''))
    expect(keys.filter((k) => !LAYOUTS[k])).toEqual([])
  })
})
