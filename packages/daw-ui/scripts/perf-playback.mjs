// Playback performance harness: the dev app on the mock backend, with the
// engine's 30 Hz transport and meter events played in, stopped and then
// playing. Prints frames per second, React commits, main-thread time, which
// components rendered and which started a render themselves.
//
//   cd packages/daw-ui && node scripts/perf-playback.mjs [keys...]   (F6, F7, F9 open panels)
//   NOCOUNT=1  skip the per-component count (it costs time itself)
//   WHY=Name   list which hooks changed when that component rendered itself
//   TOP=40     show more components
//   PROFILE=1  CPU self time per function while playing
//   TRACE=f    write a Chrome trace of the playing phase to f
//
// Headless Chromium composites in software, so its FPS runs lower than on
// a machine with a GPU: compare runs with each other, not with the app.
import { spawn } from 'node:child_process'
import { setTimeout as sleep } from 'node:timers/promises'
import { chromium } from 'playwright'
const keys = process.argv.slice(2)
const vite = spawn('npm', ['run', 'dev'], { cwd: process.cwd(), stdio: 'ignore', detached: true })
const HOOK = () => {
  const counts = new Map(); const selfc = new Map(); const why = new Map(); let commits = 0
  const nameOf = (f) => { const t = f.type; if (!t) return null; return t.displayName || t.name || (t.render && (t.render.displayName || t.render.name)) || (t.type && (t.type.displayName || t.type.name)) || 'anon' }
  const stamp = new WeakMap(); const walk = (f) => { while (f) { const fresh = stamp.get(f) !== commits - 1; stamp.set(f, commits); if (fresh && [0, 1, 11, 14, 15].includes(f.tag) && f.alternate && (f.flags & 1)) { const n = nameOf(f); counts.set(n, (counts.get(n) || 0) + 1); if (f.memoizedProps === f.alternate.memoizedProps) { selfc.set(n, (selfc.get(n) || 0) + 1); if (n === window.__hwWhy) { let h = f.memoizedState, o = f.alternate.memoizedState, i = 0; while (h && o) { if (h.memoizedState !== o.memoizedState) { const v = h.memoizedState; const k = `hook ${i}: ${typeof v === 'object' && v ? Object.keys(v).slice(0, 6).join(',') : String(v).slice(0, 40)}`; why.set(k, (why.get(k) || 0) + 1) } h = h.next; o = o.next; i++ } } } } if (f.child) walk(f.child); f = f.sibling } }
  window.__REACT_DEVTOOLS_GLOBAL_HOOK__ = { supportsFiber: true, renderers: new Map(), inject() { return 1 }, onCommitFiberRoot(_id, root) { commits++; if (window.__hwCount) walk(root.current) }, onCommitFiberUnmount() {}, onPostCommitFiberRoot() {}, checkDCE() {}, isDisabled: false }
  window.__hwPerf = { counts, selfc, why, get commits() { return commits }, reset() { counts.clear(); selfc.clear(); why.clear(); commits = 0 } }
  window.__hwFrames = 0; const tick = () => { window.__hwFrames++; requestAnimationFrame(tick) }; requestAnimationFrame(tick)
  window.__hwLong = { n: 0, ms: 0 }
  try { new PerformanceObserver((l) => { for (const e of l.getEntries()) { window.__hwLong.n++; window.__hwLong.ms += e.duration } }).observe({ entryTypes: ['longtask'] }) } catch {}
}
try {
  for (let i = 0; i < 60; i++) { try { if ((await fetch('http://localhost:5173/')).ok) break } catch {} await sleep(500) }
  const b = await chromium.launch({ executablePath: '/usr/bin/chromium-browser', args: ['--no-sandbox'] })
  const page = await b.newPage({ viewport: { width: 1536, height: 920 } })
  await page.addInitScript(HOOK); if (process.env.NOCOUNT) await page.addInitScript(() => { window.__hwNoCount = true }); await page.addInitScript((t) => { window.__hwTop = t }, process.env.TOP || '18'); if (process.env.WHY) await page.addInitScript((w) => { window.__hwWhy = w }, process.env.WHY)
  const errs = []; page.on('pageerror', (e) => errs.push(String(e)))
  await page.goto('http://localhost:5173/', { waitUntil: 'networkidle' })
  await sleep(4000)
  for (const re of ['^Close this screen$', 'SKIP']) await page.evaluate((r) => { const b = [...document.querySelectorAll('button')].find((x) => new RegExp(r, 'i').test(x.textContent.trim())); b && b.click() }, re)
  await sleep(800)
  for (const k of keys) { await page.keyboard.press(k); await sleep(700) }
  const ids = await page.evaluate(async () => (await window.__TAURI_INTERNALS__.invoke('get_tracks')).map((t) => t.id))
  const cdp = await page.context().newCDPSession(page); await cdp.send('Performance.enable')
  const metrics = async () => Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map((m) => [m.name, m.value]))
  const phase = async (label, playing) => {
    await page.evaluate(() => { window.__hwPerf.reset(); window.__hwCount = !window.__hwNoCount; window.__hwFrames = 0; window.__hwLong = { n: 0, ms: 0 } })
    const m0 = await metrics(); const t0 = Date.now()
    await page.evaluate(async ({ ids, playing }) => {
      let pos = 0; const end = performance.now() + 4000
      while (performance.now() < end) {
        if (playing) pos += 1600
        const db = () => (playing ? -30 + Math.random() * 28 : -120)
        window.__hwMockEmit('daw:meters', { peak_db: db(), peak_hold_db: -3, true_peak_db: db(), rms_db: db(), lufs_m: playing ? -9 : null, lufs_s: null, lufs_i: null, clipped: false })
        window.__hwMockEmit('daw:trackMeters', ids.map((id) => ({ id, peakL: db(), peakR: db(), rms: db(), preFaderPeak: db(), autoVolumeDb: null, autoPan: null })))
        window.__hwMockEmit('daw:transport', { position: pos, playing, bpm: 150, masterVolumeDb: 0, timeSig: [4, 4], patternMode: false, looping: false, loopStart: 0, loopEnd: 0 })
        await new Promise((r) => setTimeout(r, 33))
      }
    }, { ids, playing })
    const secs = (Date.now() - t0) / 1000; const m1 = await metrics()
    const r = await page.evaluate(() => ({ frames: window.__hwFrames, long: window.__hwLong, commits: window.__hwPerf.commits, top: [...window.__hwPerf.counts.entries()].sort((a, b) => b[1] - a[1]).slice(0, +(window.__hwTop || 18)), why: [...window.__hwPerf.why.entries()].slice(0, 10), self: [...window.__hwPerf.selfc.entries()].sort((a, b) => b[1] - a[1]).slice(0, 14) }))
    const d = (k) => ((m1[k] - m0[k]) / secs * 1000).toFixed(0)
    console.log(`\n== ${label}: fps ${(r.frames / secs).toFixed(0)}, commits/s ${(r.commits / secs).toFixed(0)}, long tasks ${r.long.n} (${r.long.ms.toFixed(0)} ms), per second: task ${d('TaskDuration')} ms, script ${d('ScriptDuration')} ms, layout ${d('LayoutDuration')} ms, style ${d('RecalcStyleDuration')} ms`)
    console.log('rendered/s:', r.top.map(([n, c]) => `${n}:${(c / secs).toFixed(0)}`).join('  '))
    if (r.why.length) console.log('why', JSON.stringify(r.why))
    console.log('started by itself/s:', r.self.map(([n, c]) => `${n}:${(c / secs).toFixed(0)}`).join('  '))
  }
  const anim = await page.evaluate(() => {
    const running = document.getAnimations().filter((a) => a.playState === 'running')
    const names = {}
    for (const a of running) { const t = a.effect && a.effect.target; const k = `${a.animationName || a.constructor.name} on ${t ? (t.className && t.className.baseVal === undefined ? String(t.className).split(' ')[0] : t.tagName) : '?'}`; names[k] = (names[k] || 0) + 1 }
    let blur = 0; for (const el of document.querySelectorAll('*')) { const cs = getComputedStyle(el); if ((cs.backdropFilter && cs.backdropFilter !== 'none') || (cs.filter && cs.filter.includes('blur'))) blur++ }
    return { running: running.length, names, blur }
  })
  console.log('running animations:', anim.running, JSON.stringify(anim.names), 'blurred elements:', anim.blur)
  await phase('stopped', false)
  if (process.env.PROFILE) { await cdp.send('Profiler.enable'); await cdp.send('Profiler.setSamplingInterval', { interval: 200 }); await cdp.send('Profiler.start') }
  if (process.env.TRACE) {
    await b.startTracing(page, { path: process.env.TRACE, categories: ['devtools.timeline', 'disabled-by-default-devtools.timeline', 'cc', 'gpu', 'viz', 'benchmark', 'blink'] })
  }
  await phase('playing', true)
  if (process.env.TRACE) await b.stopTracing()
  if (process.env.PROFILE) {
    // Self time per function, heaviest first.
    const { profile } = await cdp.send('Profiler.stop')
    const dt = {}; const byId = new Map(profile.nodes.map((n) => [n.id, n]))
    for (let i = 0; i < profile.samples.length; i++) { const n = byId.get(profile.samples[i]); const cf = n.callFrame; const k = `${cf.functionName || '(anon)'} ${cf.url.split('/').pop()}:${cf.lineNumber + 1}`; dt[k] = (dt[k] || 0) + (profile.timeDeltas[i] || 0) }
    const total = Object.values(dt).reduce((a, b) => a + b, 0)
    console.log('\nself time (playing):'); for (const [k, v] of Object.entries(dt).sort((a, b) => b[1] - a[1]).slice(0, 25)) console.log(`  ${(v / 1000).toFixed(0).padStart(6)} ms  ${(100 * v / total).toFixed(1).padStart(5)}%  ${k}`)
  }
  if (errs.length) console.log('page errors:', errs.slice(0, 3).join(' | '))
  await b.close()
} finally { try { process.kill(-vite.pid, 'SIGTERM') } catch {} }
