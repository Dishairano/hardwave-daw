import { test, expect, type Page, type Locator } from '@playwright/test'

// The panels (v0.263.0) hid two things: a maximized panel covered the top bar's menus, and the
// panels shared the plug-in picker's class names, so the picker never showed in the mixer.

async function boot(page: Page) {
  await page.goto('/')
  // The welcome screen and the first-run setup wizard sit over everything until closed.
  await page.getByText('Close this screen').click({ timeout: 10000 })
  await page.getByText(/won.t ask again/i).click({ timeout: 5000 }).catch(() => {})
}

/** Whether the element drawn at the middle of `target` belongs to it. */
async function onTop(target: Locator): Promise<boolean> {
  return target.evaluate((el) => {
    const r = el.getBoundingClientRect()
    const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2)
    return !!hit && el.contains(hit)
  })
}

async function maximizeMixer(page: Page) {
  const mixer = page.locator('[data-panel="mixer"]')
  if (!(await mixer.count())) await page.keyboard.press('F9')
  await expect(mixer).toBeVisible()
  // The title bar is the panel's first child.
  await mixer.locator(':scope > :first-child').dblclick({ position: { x: 120, y: 10 } })
  await expect(mixer).toHaveClass(/\bmax\b/)
}

test.describe('Panels and what opens over them', () => {
  test.beforeEach(async ({ page }) => boot(page))

  test('the top bar menus open above a maximized panel', async ({ page }) => {
    await maximizeMixer(page)
    await page.locator('.fl-menu').getByText('Add', { exact: true }).click()
    const rows = page.locator('.fl-menu-dropdown').first().locator('.fl-menu-row')
    expect(await rows.count()).toBeGreaterThan(1)
    expect(await onTop(rows.last()), 'the last row of the Add menu is drawn on top').toBe(true)
  })

  test('the plug-in picker opens over the mixer', async ({ page }) => {
    // The harness's shell has tracks in it; the dev build starts with none.
    await page.goto('/screenshot.html?panel=panels')
    await page.waitForTimeout(3000)
    await maximizeMixer(page)
    await page.locator('[data-panel="mixer"] .mx-strip').first().click({ position: { x: 20, y: 30 } })
    await page.getByText('+ click to add plugin').first().click()
    const flyout = page.locator('.hw-pf.open')
    await expect(flyout).toBeVisible()
    expect(await onTop(flyout), 'the picker is drawn on top').toBe(true)
  })

  test('a closed picker takes no room in a panel', async ({ page }) => {
    await maximizeMixer(page)
    const closed = page.locator('.hw-pf:not(.open)')
    for (const el of await closed.all()) await expect(el).toBeHidden()
  })
})
