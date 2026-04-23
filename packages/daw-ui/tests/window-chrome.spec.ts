import { test, expect, type Page } from '@playwright/test'

async function boot(page: Page) {
  await page.goto('/')
  await page.waitForTimeout(5000)
}

test.describe('Window chrome — drag region', () => {
  test.beforeEach(async ({ page }) => boot(page))

  test('title bar element with data-tauri-drag-region exists', async ({ page }) => {
    const dragRegion = page.locator('[data-tauri-drag-region]').first()
    await expect(dragRegion).toBeVisible()
  })

  test('page does not have an unhandled console error on boot', async ({ page }) => {
    const errors: string[] = []
    page.on('pageerror', (err) => errors.push(err.message))
    page.on('console', (msg) => {
      if (msg.type() === 'error') {
        const txt = msg.text()
        // Tauri core.js warnings allowed
        if (!txt.includes('core.js')) errors.push(txt)
      }
    })
    await page.goto('/')
    await page.waitForTimeout(5500)
    // Some Tauri APIs (invoke) naturally fail in browser preview — treat those
    // separately by filtering "window.__TAURI_INTERNALS__"-related errors.
    const fatal = errors.filter((e) => !e.includes('TAURI') && !e.includes('invoke') && !e.includes('not available'))
    expect(fatal).toEqual([])
  })
})

test.describe('Viewport / DPI robustness', () => {
  for (const vp of [
    { name: 'desktop-1080p', w: 1920, h: 1080 },
    { name: 'desktop-2k', w: 2560, h: 1440 },
    { name: 'laptop-1366', w: 1366, h: 768 },
    { name: 'tablet', w: 1024, h: 768 },
    { name: 'mobile-sm', w: 375, h: 667 },
  ]) {
    test(`renders at ${vp.name} (${vp.w}x${vp.h})`, async ({ page }) => {
      await page.setViewportSize({ width: vp.w, height: vp.h })
      await page.goto('/')
      await page.waitForTimeout(5000)
      // Must find the title bar at all resolutions.
      await expect(page.locator('[data-tauri-drag-region]').first()).toBeVisible()
    })
  }
})
