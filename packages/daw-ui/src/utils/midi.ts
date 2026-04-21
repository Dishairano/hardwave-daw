// Standard MIDI File (SMF) reader / writer — format 0/1.
// Ticks use the source file's division. Callers rescale to project PPQ.

export interface SmfNote {
  pitch: number
  velocity: number
  startTicks: number
  durationTicks: number
}

export interface SmfImport {
  ppq: number
  tracks: SmfNote[][]
}

// ── Writer ────────────────────────────────────────────────────────────────

export function encodeSingleTrackMidi(notes: SmfNote[], ppq: number, tempoBpm = 120): Blob {
  const microsPerQuarter = Math.round(60_000_000 / Math.max(1, tempoBpm))

  const events: { tick: number; bytes: number[] }[] = []
  // Tempo meta event at tick 0
  events.push({
    tick: 0,
    bytes: [0xFF, 0x51, 0x03, (microsPerQuarter >> 16) & 0xFF, (microsPerQuarter >> 8) & 0xFF, microsPerQuarter & 0xFF],
  })
  for (const n of notes) {
    const vel = Math.max(1, Math.min(127, Math.round(n.velocity)))
    events.push({ tick: n.startTicks, bytes: [0x90, n.pitch & 0x7F, vel] })
    events.push({ tick: n.startTicks + Math.max(1, n.durationTicks), bytes: [0x80, n.pitch & 0x7F, 0] })
  }
  events.sort((a, b) => a.tick - b.tick)

  const trackBytes: number[] = []
  let last = 0
  for (const e of events) {
    const delta = e.tick - last
    trackBytes.push(...vlq(delta))
    trackBytes.push(...e.bytes)
    last = e.tick
  }
  trackBytes.push(0, 0xFF, 0x2F, 0x00) // End of track

  const header = [
    0x4D, 0x54, 0x68, 0x64, // MThd
    0x00, 0x00, 0x00, 0x06,
    0x00, 0x00,             // format 0
    0x00, 0x01,             // ntrks
    (ppq >> 8) & 0xFF, ppq & 0xFF,
  ]
  const trackHdr = [
    0x4D, 0x54, 0x72, 0x6B, // MTrk
    (trackBytes.length >> 24) & 0xFF,
    (trackBytes.length >> 16) & 0xFF,
    (trackBytes.length >> 8) & 0xFF,
    trackBytes.length & 0xFF,
  ]
  const all = new Uint8Array(header.length + trackHdr.length + trackBytes.length)
  all.set(header, 0)
  all.set(trackHdr, header.length)
  all.set(trackBytes, header.length + trackHdr.length)
  return new Blob([all], { type: 'audio/midi' })
}

function vlq(v: number): number[] {
  let value = Math.max(0, Math.floor(v))
  const bytes: number[] = [value & 0x7F]
  value >>= 7
  while (value > 0) {
    bytes.push((value & 0x7F) | 0x80)
    value >>= 7
  }
  bytes.reverse()
  return bytes
}

// ── Reader ────────────────────────────────────────────────────────────────

class Reader {
  private i = 0
  constructor(private buf: Uint8Array) {}
  eof() { return this.i >= this.buf.length }
  u8() { return this.buf[this.i++] }
  u16() { return (this.buf[this.i++] << 8) | this.buf[this.i++] }
  u32() { return (this.buf[this.i++] * 0x1000000) + (this.buf[this.i++] << 16) + (this.buf[this.i++] << 8) + this.buf[this.i++] }
  bytes(n: number) { const r = this.buf.subarray(this.i, this.i + n); this.i += n; return r }
  str(n: number) { let s = ''; for (let k = 0; k < n; k++) s += String.fromCharCode(this.u8()); return s }
  vlq() {
    let v = 0
    while (true) {
      const b = this.u8()
      v = (v << 7) | (b & 0x7F)
      if ((b & 0x80) === 0) break
    }
    return v
  }
  offset() { return this.i }
  seek(i: number) { this.i = i }
}

export function decodeMidi(buf: ArrayBuffer): SmfImport {
  const r = new Reader(new Uint8Array(buf))
  const hdr = r.str(4)
  if (hdr !== 'MThd') throw new Error('Not a MIDI file')
  const hdrLen = r.u32()
  const format = r.u16()
  const ntrks = r.u16()
  const division = r.u16()
  if (division & 0x8000) throw new Error('SMPTE time division not supported')
  r.seek(8 + hdrLen)

  const tracks: SmfNote[][] = []
  for (let t = 0; t < ntrks; t++) {
    const tag = r.str(4)
    const len = r.u32()
    if (tag !== 'MTrk') { r.seek(r.offset() + len); continue }
    const end = r.offset() + len
    const notes: SmfNote[] = []
    const openNotes = new Map<number, { start: number; vel: number }>()
    let tick = 0
    let runningStatus = 0

    while (r.offset() < end) {
      const delta = r.vlq()
      tick += delta
      let status = r.u8()
      if (status < 0x80) {
        r.seek(r.offset() - 1)
        status = runningStatus
      } else {
        runningStatus = status
      }
      const type = status & 0xF0
      if (status === 0xFF) {
        r.seek(r.offset() - 1)
        r.u8() // re-read 0xFF
        r.u8() // meta type
        const l = r.vlq()
        r.bytes(l)
      } else if (status === 0xF0 || status === 0xF7) {
        const l = r.vlq()
        r.bytes(l)
      } else if (type === 0x90) {
        const pitch = r.u8()
        const vel = r.u8()
        if (vel === 0) {
          const open = openNotes.get(pitch)
          if (open) {
            notes.push({ pitch, velocity: open.vel, startTicks: open.start, durationTicks: Math.max(1, tick - open.start) })
            openNotes.delete(pitch)
          }
        } else {
          openNotes.set(pitch, { start: tick, vel })
        }
      } else if (type === 0x80) {
        const pitch = r.u8()
        r.u8()
        const open = openNotes.get(pitch)
        if (open) {
          notes.push({ pitch, velocity: open.vel, startTicks: open.start, durationTicks: Math.max(1, tick - open.start) })
          openNotes.delete(pitch)
        }
      } else if (type === 0xA0 || type === 0xB0 || type === 0xE0) {
        r.u8(); r.u8()
      } else if (type === 0xC0 || type === 0xD0) {
        r.u8()
      } else {
        break
      }
    }
    for (const [pitch, open] of openNotes) {
      notes.push({ pitch, velocity: open.vel, startTicks: open.start, durationTicks: Math.max(1, tick - open.start) })
    }
    tracks.push(notes)
    r.seek(end)
    if (format === 0) break
  }
  return { ppq: division, tracks }
}

export function rescaleNotes(notes: SmfNote[], fromPpq: number, toPpq: number): SmfNote[] {
  if (fromPpq === toPpq) return notes
  const k = toPpq / fromPpq
  return notes.map(n => ({
    pitch: n.pitch,
    velocity: n.velocity,
    startTicks: Math.round(n.startTicks * k),
    durationTicks: Math.max(1, Math.round(n.durationTicks * k)),
  }))
}
