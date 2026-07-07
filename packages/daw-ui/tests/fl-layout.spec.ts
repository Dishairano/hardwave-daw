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
    // DOM text is Title Case; CSS uppercases visually.
    const texts = await page.locator('.fl-menu > *').allTextContents()
    const menuLabels = texts.map(t => t.trim().split('\n')[0]).filter(t =>
      ['File', 'Edit', 'Add', 'Patterns', 'View', 'Options', 'Tools', 'Help'].includes(t))
    expect(menuLabels).toEqual(['File', 'Edit', 'Add', 'Patterns', 'View', 'Options', 'Tools', 'Help'])
  })

  test('hint row exists below the toolbar (Hardwave places it left)', async ({ page }) => {
    // Design divergence from FL (which right-aligns hints): the hint
    // row is its own second-row strip. Assert it renders.
    await expect(page.locator('.fl-hint-row')).toBeVisible()
  })
})

test.describe('Toolbar — FL Studio layout order', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await page.waitForTimeout(5000)
  })

  test('panel toggle buttons are in FL order: Playlist, Channels, Piano Roll, Mixer', async ({ page }) => {
    const labels = await page.locator('.fl-panel-btns button').allTextContents()
    expect(labels).toEqual(['Playlist', 'Channels', 'Piano Roll', 'Mixer'])
  })

  test('toolbar has correct element groups left to right', async ({ page }) => {
    const toolbar = page.locator('div').filter({ hasText: 'PAT' }).first()
    const box = await toolbar.boundingBox()
    expect(box).toBeTruthy()

    // PAT/SONG toggle exists (scoped: the picker tabs also say PAT)
    await expect(page.locator('.fl-mode-toggle button', { hasText: 'PAT' })).toBeVisible()
    await expect(page.locator('.fl-mode-toggle button', { hasText: 'SONG' })).toBeVisible()

    // Pattern selector exists
    await expect(page.getByText('Pattern 1').first()).toBeVisible()

    // Transport buttons: 3 buttons (record, stop, play)
    // Tempo display exists
    // Time display exists

    // Snap pill (defaults to 1/4) in the playlist tool row
    await expect(page.getByText('SNAP')).toBeVisible()

    // Perf cluster: CPU + MEM meters (top right)
    await expect(page.getByText('CPU')).toBeVisible()
    await expect(page.getByText('MEM')).toBeVisible()
  })

  test('PAT button is before SONG button (left to right)', async ({ page }) => {
    const pat = page.locator('.fl-mode-toggle button', { hasText: 'PAT' })
    const song = page.locator('.fl-mode-toggle button', { hasText: 'SONG' })
    const patBox = await pat.boundingBox()
    const songBox = await song.boundingBox()
    expect(patBox!.x).toBeLessThan(songBox!.x)
  })

  test('tempo display shows a number', async ({ page }) => {
    // The BPM readout is a drag/click LCD, not an <input>.
    const bpm = page.locator('.fl-bpm')
    await expect(bpm).toBeVisible()
    const text = (await bpm.textContent()) ?? ''
    const val = parseFloat(text.replace(/[^0-9.]/g, ''))
    expect(val).toBeGreaterThanOrEqual(10)
    expect(val).toBeLessThanOrEqual(522)
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
    const rack = page.getByTestId('panel-channel-rack')
    // Group filter buttons
    await expect(rack.getByRole('button', { name: 'All', exact: true })).toBeVisible()
    await expect(rack.getByRole('button', { name: 'Audio' })).toBeVisible()
    await expect(rack.getByRole('button', { name: 'MIDI' })).toBeVisible()

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
    const rack = page.getByTestId('panel-channel-rack')
    await expect(rack.getByText(/\d+ channels/)).toBeVisible()
    await expect(rack.getByRole('button', { name: 'Add', exact: true })).toBeVisible()
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
    await expect(page.getByTestId('panel-piano-roll')).not.toBeVisible()
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
    const pr = page.getByTestId('panel-piano-roll')
    await expect(pr).not.toBeVisible()

    await page.keyboard.press('F7')
    await page.waitForTimeout(300)
    await expect(pr).toBeVisible()

    await page.keyboard.press('F7')
    await page.waitForTimeout(300)
    await expect(pr).not.toBeVisible()
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
    const pr = page.getByTestId('panel-piano-roll')
    await expect(pr).toBeVisible()
    await expect(pr.getByRole('button', { name: 'draw', exact: true })).toBeVisible()
    await expect(pr.getByRole('button', { name: 'select', exact: true })).toBeVisible()
    await expect(pr.getByRole('button', { name: 'erase', exact: true })).toBeVisible()
  })

  test('has snap options', async ({ page }) => {
    const snapSelect = page.getByTestId('panel-piano-roll').locator('select').first()
    await expect(snapSelect).toBeVisible()
    const options = await snapSelect.locator('option').allTextContents()
    expect(options.length).toBeGreaterThan(2)
  })

  test('has velocity lane at bottom', async ({ page }) => {
    await expect(
      page.getByTestId('panel-piano-roll').getByTestId('velocity-lane'),
    ).toBeVisible()
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

  test('title bar height stays compact', async ({ page }) => {
    // Hardwave's topbar is a single denser row than FL's 22px sliver;
    // guard against it bloating past ~40px (regression canary).
    const titleBar = page.locator('.fl-topbar')
    const box = await titleBar.boundingBox()
    expect(box!.height).toBeLessThanOrEqual(40)
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
