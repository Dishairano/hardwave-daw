/**
 * Dev-only: capture the DAW UI headlessly so visual changes can be verified
 * before a build. Boots the vite dev server, loads the screenshot harness
 * (screenshot.html → src/dev/screenshot.tsx, mocked Tauri backend + seeded
 * clips), screenshots it with Playwright (system chromium), then tears down.
 *
 * Usage:  node scripts/screenshot.mjs [outfile]
 * Default outfile: /tmp/daw-shot.png
 */
import { spawn } from 'node:child_process'
import { setTimeout as sleep } from 'node:timers/promises'
import { existsSync } from 'node:fs'
import { chromium } from 'playwright'

const OUT = process.argv[2] || '/tmp/daw-shot.png'
const URL = 'http://localhost:5173/screenshot.html'
const CHROME =
  process.env.CHROMIUM_PATH ||
  ['/usr/bin/chromium-browser', '/usr/bin/chromium', '/snap/bin/chromium'].find((p) => existsSync(p))

async function waitForServer(url, ms = 30000) {
  const deadline = Date.now() + ms
  while (Date.now() < deadline) {
    try {
      const r = await fetch(url)
      if (r.ok) return true
    } catch { /* not up yet */ }
    await sleep(400)
  }
  throw new Error('vite dev server did not come up in time')
}

const vite = spawn('npm', ['run', 'dev'], { cwd: process.cwd(), stdio: 'inherit' })
let browser
try {
  await waitForServer(URL)
  browser = await chromium.launch({
    executablePath: CHROME || undefined,
    args: ['--no-sandbox', '--disable-gpu', '--force-color-profile=srgb'],
  })
  const page = await browser.newPage({ viewport: { width: 1440, height: 600, deviceScaleFactor: 2 } })
  const errors = []
  page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()) })
  page.on('pageerror', (e) => errors.push(String(e)))
  await page.goto(URL, { waitUntil: 'networkidle' })
  // Give async waveform loads + the canvas redraw a beat to settle.
  await sleep(2500)
  await page.screenshot({ path: OUT })
  console.log(`\nscreenshot → ${OUT}`)
  if (errors.length) {
    console.log(`\npage errors (${errors.length}):`)
    for (const e of errors.slice(0, 20)) console.log('  ' + e)
  }
} finally {
  if (browser) await browser.close()
  vite.kill('SIGTERM')
}
