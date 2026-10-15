import { test, expect, type Page } from '@playwright/test'

/**
 * The sample window on a sampler channel (SampleWindow.tsx, SamplerWindow)
 * over the screenshot harness, which gives the first channel the built-in
 * sampler and opens the window on it. What a control sends is read back
 * from the IPC mock's call log.
 */
interface Call { cmd: string; args?: { paramId?: number; value?: number; snapshot?: boolean; trackId?: string; slotId?: string } }

async function boot(page: Page) {
  await page.goto('/screenshot.html?panel=samplerwindow')
  await expect(page.locator('.sw-win')).toBeVisible()
}

const paramCalls = (page: Page) =>
  page.evaluate(() =>
    ((window as unknown as { __hwMockCalls: Call[] }).__hwMockCalls).filter((c) => c.cmd === 'set_plugin_parameter'),
  )

test.describe('Sample window on a sampler channel', () => {
  test.beforeEach(async ({ page }) => boot(page))

  test('shows the channel, its file and what applies to a sampler', async ({ page }) => {
    const win = page.locator('.sw-win')
    await expect(win.locator('.sw-name')).toHaveText('Industrial Kick')
    await expect(win.locator('.sw-file-name')).toHaveText('Induskick4.wav')
    // The loop knobs come with the sampler, but wait for the loop to be on.
    const loopStart = win.locator('.hwf-knob', { hasText: 'Loop start' })
    await expect(loopStart).toHaveClass(/is-off/)
    await loopStart.hover()
    await expect(page.locator('.hwf-tip')).toContainText('Use loop points')
    // Stretching is the clips', not a sampler's.
    const mul = win.locator('.hwf-knob', { hasText: 'Mul' })
    await expect(mul).toHaveClass(/is-off/)
    await mul.hover()
    await expect(page.locator('.hwf-tip')).toContainText('audio clips')
    // The envelope page is there for a sampler.
    await expect(win.locator('.sw-tab[title="Envelope"]')).toHaveClass(/is-pick/)
  })

  test('a check box sends the setting as one undo step', async ({ page }) => {
    const loop = page.locator('.sw-win .hwf-check', { hasText: 'Use loop points' })
    await loop.click()
    await expect(loop).toHaveClass(/is-on/)
    const calls = await paramCalls(page)
    expect(calls).toHaveLength(1)
    expect(calls[0].args?.slotId).toBe('hardwave.native.sampler')
    expect(calls[0].args?.paramId).toBe(11)
    expect(calls[0].args?.value).toBe(1)
    expect(calls[0].args?.snapshot).toBe(true)
    // With the loop on, its knobs turn.
    await expect(page.locator('.sw-win .hwf-knob', { hasText: 'Loop start' })).not.toHaveClass(/is-off/)
  })

  test('a knob drag sends live and snapshots once', async ({ page }) => {
    const knob = page.locator('.sw-win .hwf-knob', { hasText: 'Offset' }).locator('canvas')
    const box = await knob.boundingBox()
    if (!box) throw new Error('no knob')
    const x = box.x + box.width / 2
    const y = box.y + box.height / 2
    await page.mouse.move(x, y)
    await page.mouse.down()
    for (let i = 1; i <= 8; i++) {
      await page.mouse.move(x, y - i * 6)
      await page.waitForTimeout(30)
    }
    await page.mouse.up()
    await page.waitForTimeout(100)
    const calls = await paramCalls(page)
    expect(calls.length).toBeGreaterThanOrEqual(2)
    expect(calls[0].args?.snapshot).toBe(true)
    for (const c of calls.slice(1)) expect(c.args?.snapshot).toBe(false)
    for (const c of calls) expect(c.args?.paramId).toBe(8)
    const last = calls[calls.length - 1].args?.value as number
    expect(last).toBeGreaterThan(0)
    expect(last).toBeLessThanOrEqual(1)
  })

  test('the envelope page has the envelope, the playing modes and the root note', async ({ page }) => {
    const win = page.locator('.sw-win')
    await win.locator('.sw-tab[title="Envelope"]').click()
    await expect(win.locator('.hwf-panel-title', { hasText: 'Volume envelope' })).toBeVisible()
    await expect(win.locator('.sw-env canvas')).toBeVisible()
    // FL's defaults: the envelope off, cut itself on.
    await expect(win.locator('.hwf-check', { hasText: 'Envelope on' })).not.toHaveClass(/is-on/)
    await expect(win.locator('.hwf-check', { hasText: 'Cut itself' })).toHaveClass(/is-on/)
    // The page's one select is the root note (the sample page keeps its own).
    const root = win.locator('.sw-page:not(.is-hidden) .hwf-select')
    await expect(root).toContainText('C4')
    // Picking a root note sends it as the note's share of 127.
    await root.click()
    await win.locator('.hwf-menu button', { hasText: /^C5$/ }).click()
    const calls = await paramCalls(page)
    expect(calls).toHaveLength(1)
    expect(calls[0].args?.paramId).toBe(16)
    expect(calls[0].args?.value).toBeCloseTo(72 / 127, 6)
    await expect(root).toContainText('C5')
    // Sustain reads as a share.
    await win.locator('.hwf-knob', { hasText: 'Sustain' }).hover()
    await expect(page.locator('.hwf-tip')).toHaveText('Sustain 100 %')
  })

  test('load regions is the sampler\'s setting', async ({ page }) => {
    await page.locator('.sw-win .hwf-check', { hasText: 'Load regions' }).click()
    const calls = await paramCalls(page)
    expect(calls).toHaveLength(1)
    expect(calls[0].args?.paramId).toBe(24)
    expect(calls[0].args?.value).toBe(1)
    await page.locator('.sw-win .hwf-check', { hasText: 'Load regions' }).hover()
    await expect(page.locator('.hwf-tip')).toHaveText('The file has 2 regions and loops')
  })

  test('the sample\'s effects are the sampler\'s settings', async ({ page }) => {
    const win = page.locator('.sw-win')
    await win.locator('.hwf-check', { hasText: /^Reverse$/ }).click()
    await win.locator('.hwf-check', { hasText: 'Normalize' }).click()
    const calls = await paramCalls(page)
    expect(calls.map((c) => [c.args?.paramId, c.args?.value])).toEqual([[28, 1], [26, 1]])
    await expect(win.locator('.hwf-check', { hasText: /^Reverse$/ })).toHaveClass(/is-on/)
  })

  test('Escape closes it', async ({ page }) => {
    await page.keyboard.press('Escape')
    await expect(page.locator('.sw-win')).toHaveCount(0)
  })
})
