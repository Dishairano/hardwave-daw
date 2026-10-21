import { describe, expect, it } from 'vitest'
import { readKey, shareRead } from './sharedReads'

function deferred<T>() {
  let resolve!: (v: T) => void
  let reject!: (e: unknown) => void
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej })
  return { promise, resolve, reject }
}

describe('shared reads', () => {
  it('shares one request between polls while it is unanswered', async () => {
    const answer = deferred<number>()
    let started = 0
    const start = () => { started++; return answer.promise }
    const key = readKey('get_audio_load')
    const first = shareRead(key, start)
    const second = shareRead(key, start)
    expect(started).toBe(1)
    answer.resolve(42)
    expect(await first).toBe(42)
    expect(await second).toBe(42)
  })

  it('sends a new request once the last one was answered', async () => {
    let started = 0
    const start = () => { started++; return Promise.resolve(started) }
    const key = readKey('get_track_load')
    expect(await shareRead(key, start)).toBe(1)
    expect(await shareRead(key, start)).toBe(2)
  })

  it('sends a new request after a failed one', async () => {
    let started = 0
    const key = readKey('get_gain_reduction')
    await expect(shareRead(key, () => { started++; return Promise.reject(new Error('busy')) })).rejects.toThrow('busy')
    await shareRead(key, () => { started++; return Promise.resolve(0) })
    expect(started).toBe(2)
  })

  it('keeps calls with other arguments apart', () => {
    expect(readKey('get_master_samples', { nFrames: 1024 })).not.toBe(readKey('get_master_samples', { nFrames: 2048 }))
  })

  it('never shares a call that changes something', () => {
    expect(readKey('take_sandbox_crashes')).toBeNull()
    expect(readKey('drain_plugin_knob_moves')).toBeNull()
    expect(readKey('set_audio_config', { bufferSize: 256 })).toBeNull()
    expect(readKey('autosave_save')).toBeNull()
  })

  it('shares the listed reads that are not named get_', () => {
    expect(readKey('process_memory')).not.toBeNull()
    expect(readKey('collab_status')).not.toBeNull()
  })

  it('never shares a call with options', () => {
    expect(readKey('get_audio_load', undefined, { headers: { a: 'b' } })).toBeNull()
  })
})
