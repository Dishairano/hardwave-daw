/**
 * Dev-only: capture DAW panels headlessly so visual changes can be verified
 * before a build. Boots the vite dev server, loads the screenshot harness
 * (screenshot.html → src/dev/screenshot.tsx, mocked Tauri backend + seeded
 * data), screenshots each panel with Playwright (system chromium), tears down.
 *
 * Usage:
 *   node scripts/screenshot.mjs                 # sweep all panels → /tmp/daw-<panel>.png
 *   node scripts/screenshot.mjs playlist out.png  # one panel to a file
 *
 * Panels: playlist | mixer | channelrack | pianoroll
 */
import { spawn } from 'node:child_process'
import { setTimeout as sleep } from 'node:timers/promises'
import { existsSync } from 'node:fs'
import { chromium } from 'playwright'

const ALL = ['playlist', 'mixer', 'channelrack', 'pianoroll']
const arg = process.argv[2]
const single = arg && ALL.includes(arg)
const panels = single ? [arg] : ALL
const singleOut = single ? process.argv[3] : null

const BASE = 'http://localhost:5173/screenshot.html'
const CHROME =
  process.env.CHROMIUM_PATH ||
  ['/usr/bin/chromium-browser', '/usr/bin/chromium', '/snap/bin/chromium'].find((p) => existsSync(p))

async function waitForServer(url, ms = 30000) {
  const deadline = Date.now() + ms
  while (Date.now() < deadline) {
    try { if ((await fetch(url)).ok) return } catch { /* not up */ }
    await sleep(400)
  }
  throw new Error('vite dev server did not come up in time')
}

const vite = spawn('npm', ['run', 'dev'], { cwd: process.cwd(), stdio: 'inherit' })
let browser
try {
  await waitForServer(BASE)
  browser = await chromium.launch({
    executablePath: CHROME || undefined,
    args: ['--no-sandbox', '--disable-gpu', '--force-color-profile=srgb'],
  })
  for (const panel of panels) {
    const out = singleOut || `/tmp/daw-${panel}.png`
    const page = await browser.newPage({ viewport: { width: 1440, height: 720, deviceScaleFactor: 2 } })
    const errors = []
    page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()) })
    page.on('pageerror', (e) => errors.push(String(e)))
    await page.goto(`${BASE}?panel=${panel}`, { waitUntil: 'networkidle' })
    await sleep(2200) // let async loads + canvas redraw settle
    await page.screenshot({ path: out })
    console.log(`screenshot → ${out}${errors.length ? `  (${errors.length} page errors)` : ''}`)
    for (const e of errors.slice(0, 6)) console.log('    ! ' + e)
    await page.close()
  }
} finally {
  if (browser) await browser.close()
  vite.kill('SIGTERM')
}
