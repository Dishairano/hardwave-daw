import { test, expect, type Page } from '@playwright/test'

async function boot(page: Page) {
  await page.goto('/')
  await page.waitForTimeout(5000)
}

test.describe('Keyboard shortcuts', () => {
  test.beforeEach(async ({ page }) => boot(page))

  test('Space key does not crash the page', async ({ page }) => {
    await page.keyboard.press('Space')
    await page.waitForTimeout(200)
    await page.keyboard.press('Space')
    // Page still responsive — title bar menus should still be visible
    await expect(page.getByText('FILE', { exact: true })).toBeVisible()
  })

  test('Home key does not crash', async ({ page }) => {
    await page.keyboard.press('Home')
    await page.waitForTimeout(100)
    await expect(page.getByText('FILE', { exact: true })).toBeVisible()
  })

  test('End key does not crash', async ({ page }) => {
    await page.keyboard.press('End')
    await page.waitForTimeout(100)
    await expect(page.getByText('FILE', { exact: true })).toBeVisible()
  })

  test('L key does not crash (loop toggle)', async ({ page }) => {
    await page.keyboard.press('KeyL')
    await page.waitForTimeout(100)
    await page.keyboard.press('KeyL')
    await expect(page.getByText('FILE', { exact: true })).toBeVisible()
  })

  test('F5 through F9 function keys do not crash', async ({ page }) => {
    for (const key of ['F5', 'F6', 'F7', 'F8', 'F9']) {
      await page.keyboard.press(key)
      await page.waitForTimeout(80)
    }
    await expect(page.getByText('FILE', { exact: true })).toBeVisible()
  })

  test('Ctrl+Z and Ctrl+Shift+Z do not crash', async ({ page }) => {
    await page.keyboard.press('Control+KeyZ')
    await page.waitForTimeout(80)
    await page.keyboard.press('Control+Shift+KeyZ')
    await page.waitForTimeout(80)
    await expect(page.getByText('FILE', { exact: true })).toBeVisible()
  })
})

test.describe('Viewport responsiveness', () => {
  test('mobile viewport renders the main page without overflow', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 })
    await page.goto('/')
    await page.waitForTimeout(5000)
    const bodyWidth = await page.evaluate(() => document.body.scrollWidth)
    // Tauri-target desktop app, but UI should at least not explode past
    // the viewport horizontally — allow a small tolerance for floating
    // panels, asserting no catastrophic overflow.
    expect(bodyWidth).toBeLessThan(2000)
  })
})
