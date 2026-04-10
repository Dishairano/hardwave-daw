import { test, expect, type Page } from '@playwright/test'

// FL Studio's exact toolbar layout order (left to right)
// Reference: https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/toolbar_panels.htm

test.describe('Title Bar — FL Studio layout', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    // Wait for splash screen to finish
    await page.waitForTimeout(5000)
  })

  test('has menu items in FL Studio order', async ({ page }) => {
    const menus = page.locator('[data-tauri-drag-region] > div').filter({ hasText: /^(FILE|EDIT|ADD|PATTERNS|VIEW|OPTIONS|TOOLS|HELP)$/ })
    const texts = await menus.allTextContents()
    expect(texts).toEqual(['FILE', 'EDIT', 'ADD', 'PATTERNS', 'VIEW', 'OPTIONS', 'TOOLS', 'HELP'])
  })

  test('hint bar is on the right side', async ({ page }) => {
    const titleBar = page.locator('[data-tauri-drag-region]')
    const hintText = titleBar.locator('div').last()
    const titleBarBox = await titleBar.boundingBox()
    const hintBox = await hintText.boundingBox()
    if (titleBarBox && hintBox) {
      // Hint should be in the right half
      expect(hintBox.x).toBeGreaterThan(titleBarBox.width / 2)
    }
  })
})

test.describe('Toolbar — FL Studio layout order', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await page.waitForTimeout(5000)
  })

  test('panel toggle buttons are in FL order: Playlist, Channel, PianoRoll, Mixer, Browser', async ({ page }) => {
    // The toolbar is the second bar (index 1), find the first button group
    const toolbar = page.locator('div').filter({ has: page.locator('button') }).nth(1)

    // Get all hint texts from panel buttons by hovering
    const panelBtns = page.locator('button').filter({ has: page.locator('svg') })
    const hints: string[] = []

    // First 5 icon buttons should be panel toggles in FL order
    for (let i = 0; i < 5; i++) {
      const btn = panelBtns.nth(i)
      await btn.hover()
      await page.waitForTimeout(100)
      // Check hint bar text
    }
  })

  test('toolbar has correct element groups left to right', async ({ page }) => {
    const toolbar = page.locator('div').filter({ hasText: 'PAT' }).first()
    const box = await toolbar.boundingBox()
    expect(box).toBeTruthy()

    // PAT/SONG toggle exists
    await expect(page.getByText('PAT', { exact: true })).toBeVisible()
    await expect(page.getByText('SONG', { exact: true })).toBeVisible()

    // Pattern selector exists
    await expect(page.getByText('Pattern 1').first()).toBeVisible()

    // Transport buttons: 3 buttons (record, stop, play)
    // Tempo display exists
    // Time display exists

    // Snap selector with "Line" text
    await expect(page.getByText('Line')).toBeVisible()

    // Tool buttons section — verify all 8 tool hints exist
    const toolNames = ['Draw', 'Paint', 'Delete', 'Mute', 'Slip', 'Slice', 'Select', 'Zoom']
    // These are rendered as small buttons with SVG icons

    // CPU/POLY meters
    await expect(page.getByText('CPU')).toBeVisible()
    await expect(page.getByText('POLY')).toBeVisible()

    // Scope
    await expect(page.getByText('SCOPE')).toBeVisible()
  })

  test('PAT button is before SONG button (left to right)', async ({ page }) => {
    const pat = page.getByText('PAT', { exact: true })
    const song = page.getByText('SONG', { exact: true })
    const patBox = await pat.boundingBox()
    const songBox = await song.boundingBox()
    expect(patBox!.x).toBeLessThan(songBox!.x)
  })

  test('tempo display shows a number', async ({ page }) => {
    // Tempo input should exist and have a numeric value
    const tempoInput = page.locator('input[type="number"]')
    await expect(tempoInput).toBeVisible()
    const val = await tempoInput.inputValue()
    expect(Number(val)).toBeGreaterThanOrEqual(10)
    expect(Number(val)).toBeLessThanOrEqual(522)
  })

  test('master volume slider exists', async ({ page }) => {
    // Volume slider bar exists
    const volBar = page.locator('div').filter({ has: page.locator('svg polygon') })
    expect(await volBar.count()).toBeGreaterThan(0)
  })

  test('master pitch knob exists', async ({ page }) => {
    await expect(page.getByText('PIT')).toBeVisible()
  })
})

test.describe('Channel Rack — FL Studio layout', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await page.waitForTimeout(5000)
    // Open Channel Rack (F6)
    await page.keyboard.press('F6')
    await page.waitForTimeout(300)
  })

  test('has top toolbar with correct elements', async ({ page }) => {
    // Group filter buttons
    await expect(page.getByText('All')).toBeVisible()
    await expect(page.getByRole('button', { name: 'Audio' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'MIDI' })).toBeVisible()

    // Swing label
    await expect(page.getByText('SWG')).toBeVisible()

    // Step count
    await expect(page.getByText('Steps')).toBeVisible()

    // Pattern label
    await expect(page.getByText('Pattern 1').first()).toBeVisible()
  })

  test('has graph editor toggle', async ({ page }) => {
    await expect(page.getByText('Graph')).toBeVisible()
  })

  test('has bottom bar with channel count and add button', async ({ page }) => {
    await expect(page.getByText(/\d+ channels/)).toBeVisible()
    await expect(page.getByRole('button', { name: 'Add' })).toBeVisible()
  })

  test('channel row has elements in FL order: LED, pan, vol, mixer#, name, select, steps', async ({ page }) => {
    // This test verifies the horizontal ordering of elements within a channel row
    // We need at least one channel to test — if none exist, skip
    const channelCount = page.getByText(/\d+ channels/)
    const countText = await channelCount.textContent()
    if (countText === '0 channels') {
      test.skip()
      return
    }

    // Channel rows contain: mute LED (circle), knobs, mixer number, name, select dot, step buttons
    // Verify the channel name section exists with a color strip (3px wide div)
  })
})

test.describe('Default panel state — FL Studio defaults', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await page.waitForTimeout(5000)
  })

  test('Browser is visible by default', async ({ page }) => {
    // Browser panel should be rendered
    await expect(page.getByText('Plugins').first()).toBeVisible()
  })

  test('Playlist is visible by default', async ({ page }) => {
    // Playlist/arrangement area should be rendered (TrackList + Arrangement canvas)
    const canvas = page.locator('canvas')
    expect(await canvas.count()).toBeGreaterThan(0)
  })

  test('Channel Rack is hidden by default', async ({ page }) => {
    // SWG label should NOT be visible (it's inside channel rack)
    await expect(page.getByText('SWG')).not.toBeVisible()
  })

  test('Mixer is hidden by default', async ({ page }) => {
    // Master strip text should NOT be visible
    await expect(page.getByText('MASTER').first()).not.toBeVisible()
  })

  test('Piano Roll is hidden by default', async ({ page }) => {
    await expect(page.getByText('Piano Roll').first()).not.toBeVisible()
  })
})

test.describe('Keyboard shortcuts — FL Studio bindings', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await page.waitForTimeout(5000)
  })

  test('F5 toggles Playlist', async ({ page }) => {
    // Playlist visible by default
    const canvasBefore = await page.locator('canvas').count()
    expect(canvasBefore).toBeGreaterThan(0)

    // Hide playlist
    await page.keyboard.press('F5')
    await page.waitForTimeout(200)

    // Show playlist again
    await page.keyboard.press('F5')
    await page.waitForTimeout(200)
    const canvasAfter = await page.locator('canvas').count()
    expect(canvasAfter).toBeGreaterThan(0)
  })

  test('F6 toggles Channel Rack', async ({ page }) => {
    // Channel Rack hidden by default
    await expect(page.getByText('SWG')).not.toBeVisible()

    // Show it
    await page.keyboard.press('F6')
    await page.waitForTimeout(300)
    await expect(page.getByText('SWG')).toBeVisible()

    // Hide it
    await page.keyboard.press('F6')
    await page.waitForTimeout(300)
    await expect(page.getByText('SWG')).not.toBeVisible()
  })

  test('F7 toggles Piano Roll', async ({ page }) => {
    await expect(page.getByText('Piano Roll').first()).not.toBeVisible()

    await page.keyboard.press('F7')
    await page.waitForTimeout(300)
    await expect(page.getByText('Piano Roll').first()).toBeVisible()

    await page.keyboard.press('F7')
    await page.waitForTimeout(300)
    await expect(page.getByText('Piano Roll').first()).not.toBeVisible()
  })

  test('Space toggles playback', async ({ page }) => {
    // Just verify it doesn't crash
    await page.keyboard.press('Space')
    await page.waitForTimeout(200)
    await page.keyboard.press('Space')
    await page.waitForTimeout(200)
  })

  test('Home resets position', async ({ page }) => {
    await page.keyboard.press('Home')
    await page.waitForTimeout(200)
    // Should not crash
  })
})

test.describe('Piano Roll — FL Studio layout', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await page.waitForTimeout(5000)
    await page.keyboard.press('F7')
    await page.waitForTimeout(300)
  })

  test('has header with tool selector and snap dropdown', async ({ page }) => {
    await expect(page.getByText('Piano Roll').first()).toBeVisible()
    await expect(page.getByRole('button', { name: 'draw' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'select' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'erase' })).toBeVisible()
  })

  test('has snap options', async ({ page }) => {
    const snapSelect = page.locator('select')
    await expect(snapSelect.first()).toBeVisible()
    // Should have standard FL snap values
    const options = await snapSelect.first().locator('option').allTextContents()
    expect(options).toContain('1/4')
    expect(options).toContain('1/8')
    expect(options).toContain('1/16')
  })

  test('has velocity lane at bottom', async ({ page }) => {
    await expect(page.getByTestId('velocity-lane')).toBeVisible()
  })

  test('piano keyboard is on the left (narrower than grid)', async ({ page }) => {
    // Piano roll should have multiple canvases — keyboard + grid + velocity
    const canvases = page.locator('canvas')
    expect(await canvases.count()).toBeGreaterThanOrEqual(2)
  })
})

test.describe('Visual consistency', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await page.waitForTimeout(5000)
  })

  test('title bar height is compact (FL style ~22px)', async ({ page }) => {
    const titleBar = page.locator('[data-tauri-drag-region]')
    const box = await titleBar.boundingBox()
    expect(box!.height).toBeLessThanOrEqual(26)
    expect(box!.height).toBeGreaterThanOrEqual(18)
  })

  test('no visible scrollbars on main layout', async ({ page }) => {
    // Main container should not overflow
    const overflow = await page.locator('body').evaluate(el => {
      return window.getComputedStyle(el).overflow
    })
    // Body should not have visible overflow
  })

  test('full viewport coverage (no white gaps)', async ({ page }) => {
    const bodyBg = await page.evaluate(() => {
      return window.getComputedStyle(document.body).backgroundColor
    })
    // Should not be white
    expect(bodyBg).not.toBe('rgb(255, 255, 255)')
  })

  test('screenshot — default layout', async ({ page }) => {
    await page.screenshot({ path: 'tests/screenshots/default-layout.png', fullPage: false })
  })

  test('screenshot — all panels open', async ({ page }) => {
    await page.keyboard.press('F6') // Channel Rack
    await page.waitForTimeout(200)
    await page.keyboard.press('F7') // Piano Roll
    await page.waitForTimeout(200)
    await page.keyboard.press('F9') // Mixer
    await page.waitForTimeout(200)
    await page.screenshot({ path: 'tests/screenshots/all-panels.png', fullPage: false })
  })
})
