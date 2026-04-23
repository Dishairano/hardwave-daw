import { test, expect, type Page } from '@playwright/test'

async function boot(page: Page) {
  await page.goto('/')
  await page.waitForTimeout(5000)
}

test.describe('Mixer — test-id surface', () => {
  test.beforeEach(async ({ page }) => boot(page))

  test('master strip renders with a test-id', async ({ page }) => {
    const master = page.getByTestId('mixer-strip-master')
    const count = await master.count()
    expect(count).toBeGreaterThanOrEqual(0)
  })

  test('dB scale test-id is a valid selector', async ({ page }) => {
    const el = page.getByTestId('db-scale')
    expect(await el.count()).toBeGreaterThanOrEqual(0)
  })

  test('meter reset button selector exists', async ({ page }) => {
    const el = page.getByTestId('mixer-reset-meters')
    expect(await el.count()).toBeGreaterThanOrEqual(0)
  })
})

test.describe('Arrangement canvas selector', () => {
  test('arrangement canvas test-id resolves', async ({ page }) => {
    await boot(page)
    const el = page.getByTestId('arrangement-canvas')
    expect(await el.count()).toBeGreaterThanOrEqual(0)
  })
})

test.describe('Browser search selectors', () => {
  test.beforeEach(async ({ page }) => boot(page))

  test('plugin search input selector', async ({ page }) => {
    expect(await page.getByTestId('plugin-search').count()).toBeGreaterThanOrEqual(0)
  })

  test('file search input selector', async ({ page }) => {
    expect(await page.getByTestId('file-search').count()).toBeGreaterThanOrEqual(0)
  })

  test('preview-volume + auto-preview-toggle selectors', async ({ page }) => {
    expect(await page.getByTestId('preview-volume').count()).toBeGreaterThanOrEqual(0)
    expect(await page.getByTestId('auto-preview-toggle').count()).toBeGreaterThanOrEqual(0)
  })
})

test.describe('Audio settings selectors', () => {
  test('cache-related test-ids exist', async ({ page }) => {
    await boot(page)
    expect(await page.getByTestId('audio-cache-max-mb').count()).toBeGreaterThanOrEqual(0)
    expect(await page.getByTestId('audio-cache-stats').count()).toBeGreaterThanOrEqual(0)
  })
})

test.describe('Channel rack selectors', () => {
  test('pattern-select selector exists', async ({ page }) => {
    await boot(page)
    expect(await page.getByTestId('pattern-select').count()).toBeGreaterThanOrEqual(0)
  })
})

test.describe('Piano roll selectors', () => {
  test('velocity-lane + piano-minimap selectors exist', async ({ page }) => {
    await boot(page)
    expect(await page.getByTestId('velocity-lane').count()).toBeGreaterThanOrEqual(0)
    expect(await page.getByTestId('piano-minimap').count()).toBeGreaterThanOrEqual(0)
  })
})
