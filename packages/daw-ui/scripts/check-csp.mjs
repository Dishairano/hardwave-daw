// Serve the production bundle under the app's content security policy
// (from src-tauri/tauri.conf.json) and fail on anything it blocks, so a
// change that only works with the policy off cannot slip in.
//
//   npm run build && node scripts/check-csp.mjs
import http from 'node:http'
import fs from 'node:fs'
import path from 'node:path'
import { chromium } from 'playwright'
const conf = JSON.parse(fs.readFileSync(new URL('../../../src-tauri/tauri.conf.json', import.meta.url), 'utf8'))
const csp = Object.entries(conf.app.security.csp).map(([k, v]) => `${k} ${v}`).join('; ')
const root = path.resolve(new URL('../dist', import.meta.url).pathname)
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.woff2': 'font/woff2', '.woff': 'font/woff', '.svg': 'image/svg+xml', '.png': 'image/png' }
const server = http.createServer((req, res) => {
  let p = path.join(root, decodeURIComponent(req.url.split('?')[0]))
  if (!p.startsWith(root) || !fs.existsSync(p) || fs.statSync(p).isDirectory()) p = path.join(root, 'index.html')
  res.writeHead(200, { 'Content-Type': types[path.extname(p)] || 'application/octet-stream', 'Content-Security-Policy': csp })
  fs.createReadStream(p).pipe(res)
}).listen(4178, '127.0.0.1')
// CI installs Playwright's own Chromium; elsewhere the system one is used.
const systemChromium = ['/usr/bin/chromium-browser', '/usr/bin/chromium', '/snap/bin/chromium'].find((p) => fs.existsSync(p))
const executablePath = process.env.CHROMIUM_PATH || (process.env.CI ? undefined : systemChromium)
const browser = await chromium.launch({ executablePath, args: ['--no-sandbox'] })
const page = await browser.newPage()
const blocked = []
page.on('console', m => { if (/Content Security Policy|Refused to/.test(m.text())) blocked.push(m.text().slice(0, 220)) })
await page.goto('http://127.0.0.1:4178/', { waitUntil: 'networkidle' }).catch(() => {})
await page.waitForTimeout(3000)
const fonts = await page.evaluate(() => [...document.fonts].filter(f => f.status === 'loaded').map(f => f.family + ' ' + f.weight))
console.log('CSP violations:', blocked.length); for (const b of blocked) console.log('  ' + b)
console.log('fonts loaded:', [...new Set(fonts)].join(', ') || '(none yet)')
await browser.close(); server.close()
process.exit(blocked.length ? 1 : 0)
