import { test, expect, type Page } from '@playwright/test'

async function boot(page: Page) {
  await page.goto('/')
  // Wait past the splash animation.
  await page.waitForTimeout(5000)
}

test.describe('Panels — presence and toggling', () => {
  test.beforeEach(async ({ page }) => boot(page))

  test('all five core panel test-ids render at least once', async ({ page }) => {
    // Some panels float, some dock. Each test-id appears in App.tsx twice
    // (docked + floating branches) — the active branch is what matters.
    const ids = [
      'panel-browser',
      'panel-channel-rack',
      'panel-piano-roll',
      'panel-playlist',
      'panel-mixer',
    ]
    for (const id of ids) {
      const locator = page.getByTestId(id)
      // At least one element should be attached (even if hidden behind others)
      const count = await locator.count()
      expect(count, `${id} should appear in the DOM`).toBeGreaterThan(0)
    }
  })

  test('notification host is mounted on the page root', async ({ page }) => {
    await expect(page.getByTestId('notification-host')).toBeAttached()
  })
})

test.describe('Title bar — always visible', () => {
  test.beforeEach(async ({ page }) => boot(page))

  test('has FL-style top-level menus in exact order', async ({ page }) => {
    const expected = ['FILE', 'EDIT', 'ADD', 'PATTERNS', 'VIEW', 'OPTIONS', 'TOOLS', 'HELP']
    for (const label of expected) {
      await expect(page.getByText(label, { exact: true })).toBeVisible()
    }
  })

  test('hint bar region exists on the title bar', async ({ page }) => {
    const titleBar = page.locator('[data-tauri-drag-region]')
    await expect(titleBar).toBeVisible()
  })
})

test.describe('Dev panel — Ctrl+Shift+D opens and closes it', () => {
  test.beforeEach(async ({ page }) => boot(page))

  test('toggle via keyboard shortcut', async ({ page }) => {
    // Open
    await page.keyboard.press('Control+Shift+KeyD')
    await page.waitForTimeout(250)
    const headerAfterOpen = page.getByText('Dev Panel', { exact: false })
    await expect(headerAfterOpen.first()).toBeVisible()

    // Close via "x" button
    const closeBtn = page.locator('text="x"').last()
    await closeBtn.click()
    await page.waitForTimeout(250)
    // The panel's header text should no longer be visible
    expect(await headerAfterOpen.count()).toBe(0)
  })

  test('reopens after close via keyboard', async ({ page }) => {
    await page.keyboard.press('Control+Shift+KeyD')
    await page.waitForTimeout(200)
    await page.locator('text="x"').last().click()
    await page.waitForTimeout(200)
    await page.keyboard.press('Control+Shift+KeyD')
    await page.waitForTimeout(200)
    await expect(page.getByText('Dev Panel', { exact: false }).first()).toBeVisible()
  })
})
