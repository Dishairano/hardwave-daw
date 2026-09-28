import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../../theme'

/**
 * A small waveform beside a file in the browser.
 *
 * The browser listed sample packs as names, so picking a kick out of two
 * hundred meant auditioning them one at a time. The shape of a sound says
 * most of it at a glance: which is the long tail, which is the short click,
 * which file is silence someone left in the pack.
 *
 * Drawn only once the row is actually on screen, because a folder of two
 * hundred samples would otherwise decode two hundred files to show pictures
 * nobody scrolled to. The result is kept for the session, so scrolling back
 * costs nothing.
 */
const CACHE = new Map<string, [number, number][]>()
const WIDTH = 64
const HEIGHT = 14
const BUCKETS = 48

export function FileWaveform({ path }: { path: string }) {
  const holder = useRef<HTMLDivElement | null>(null)
  const [peaks, setPeaks] = useState<[number, number][] | null>(() => CACHE.get(path) ?? null)
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    const cached = CACHE.get(path)
    if (cached) { setPeaks(cached); return }
    const node = holder.current
    if (!node) return
    let cancelled = false
    const observer = new IntersectionObserver(entries => {
      if (!entries.some(e => e.isIntersecting)) return
      observer.disconnect()
      invoke<[number, number][]>('get_file_peaks', { filePath: path, numBuckets: BUCKETS })
        .then(result => {
          if (cancelled) return
          CACHE.set(path, result)
          setPeaks(result)
        })
        .catch(() => { if (!cancelled) setFailed(true) })
    }, { rootMargin: '80px' })
    observer.observe(node)
    return () => { cancelled = true; observer.disconnect() }
  }, [path])

  if (failed) return null

  return (
    <div
      ref={holder}
      style={{ width: WIDTH, height: HEIGHT, flexShrink: 0, opacity: peaks ? 1 : 0.25 }}
      aria-hidden
    >
      {peaks && (
        <svg width={WIDTH} height={HEIGHT} style={{ display: 'block' }}>
          {peaks.map(([low, high], i) => {
            const x = (i / Math.max(1, peaks.length - 1)) * (WIDTH - 1)
            const mid = HEIGHT / 2
            const top = mid - Math.min(1, Math.abs(high)) * (HEIGHT / 2 - 1)
            const bottom = mid + Math.min(1, Math.abs(low)) * (HEIGHT / 2 - 1)
            return (
              <line
                key={i}
                x1={x} x2={x} y1={top} y2={Math.max(bottom, top + 0.5)}
                stroke={hw.textFaint} strokeWidth={1}
              />
            )
          })}
        </svg>
      )}
    </div>
  )
}
