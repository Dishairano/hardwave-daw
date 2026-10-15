import { test, expect, type Page } from '@playwright/test'

/**
 * The sample window (SampleWindow.tsx) over the screenshot harness, which
 * seeds a session and opens the window on its first audio clip. What a
 * control sends is read back from the IPC mock's call log.
 */
interface Call { cmd: string; args?: { settings?: Record<string, unknown>; snapshot?: boolean; trackId?: string; clipId?: string } }

async function boot(page: Page) {
  await page.goto('/screenshot.html?panel=samplewindow')
  await expect(page.locator('.sw-win')).toBeVisible()
}

const settingsCalls = (page: Page) =>
  page.evaluate(() =>
    ((window as unknown as { __hwMockCalls: Call[] }).__hwMockCalls).filter((c) => c.cmd === 'set_sample_settings'),
  )

test.describe('Sample window', () => {
  test.beforeEach(async ({ page }) => boot(page))

  test('shows the clip, its file and its settings', async ({ page }) => {
    const win = page.locator('.sw-win')
    await expect(win.locator('.sw-name')).toHaveText('Induskick4 – Auto')
    await expect(win.locator('.sw-file-name')).toHaveText('Crash #1.wav')
    // The harness seeds remove DC on; normalise is off.
    await expect(win.locator('.hwf-check', { hasText: 'Remove DC offset' })).toHaveClass(/is-on/)
    await expect(win.locator('.hwf-check', { hasText: 'Normalize' })).not.toHaveClass(/is-on/)
    // What is not built yet is switched off, and says so.
    await expect(win.locator('.hwf-check', { hasText: 'Keep on disk' })).toBeDisabled()
    // The loop knobs come with sampler channels.
    const xfade = win.locator('.hwf-knob', { hasText: 'X-fade' })
    await expect(xfade).toHaveClass(/is-off/)
    await xfade.hover()
    await expect(page.locator('.hwf-tip')).toContainText('sampler channels')
  })

  test('the sample\'s window, its trim, fade stereo and declicking are the sample\'s', async ({ page }) => {
    const win = page.locator('.sw-win')
    await win.locator('.hwf-check', { hasText: 'Fade stereo' }).click()
    await win.locator('.sw-row', { hasText: 'Declicking mode' }).locator('.hwf-select').click()
    await win.locator('.hwf-menu button', { hasText: 'Smooth' }).click()
    const trim = win.locator('.hwf-knob', { hasText: 'Trim' })
    await trim.hover()
    await expect(page.locator('.hwf-tip')).toHaveText('Trim off')
    const calls = await settingsCalls(page)
    expect(calls.map((c) => c.args?.settings)).toEqual([{ fadeStereo: true }, { declick: 'Smooth' }])
    for (const c of calls) expect(c.args?.snapshot).toBe(true)
  })

  test('load regions and slice markers read the file and say what it holds', async ({ page }) => {
    const win = page.locator('.sw-win')
    await win.locator('.hwf-check', { hasText: 'Load regions' }).click()
    await expect(win.locator('.hwf-check', { hasText: 'Load regions' })).toHaveClass(/is-on/)
    await win.locator('.hwf-check', { hasText: 'Load slice markers' }).click()
    const calls = await settingsCalls(page)
    expect(calls.map((c) => c.args?.settings)).toEqual([{ loadRegions: true }, { loadSlices: true }])
    const asked = await page.evaluate(() =>
      ((window as unknown as { __hwMockCalls: Call[] }).__hwMockCalls).filter((c) => c.cmd === 'get_sample_markers').length,
    )
    expect(asked).toBeGreaterThan(0)
    await win.locator('.hwf-check', { hasText: 'Load slice markers' }).hover()
    await expect(page.locator('.hwf-tip')).toHaveText('The file has 1 marker')
  })

  test('a check box sends the setting for the sample as one undo step', async ({ page }) => {
    await page.locator('.sw-win .hwf-check', { hasText: 'Normalize' }).click()
    await expect(page.locator('.sw-win .hwf-check', { hasText: 'Normalize' })).toHaveClass(/is-on/)
    const calls = await settingsCalls(page)
    expect(calls).toHaveLength(1)
    expect(calls[0].args?.trackId).toBe('t-kick')
    expect(calls[0].args?.settings).toEqual({ normalize: true })
    expect(calls[0].args?.snapshot).toBe(true)
    // The session reloads and the window stays on its clip.
    await expect(page.locator('.sw-win')).toBeVisible()
  })

  test('a knob drag sends live and snapshots once', async ({ page }) => {
    const knob = page.locator('.sw-top .hwf-knob', { hasText: 'Vol' }).locator('canvas')
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
    const calls = await settingsCalls(page)
    expect(calls.length).toBeGreaterThanOrEqual(2)
    expect(calls[0].args?.snapshot).toBe(true)
    for (const c of calls.slice(1)) expect(c.args?.snapshot).toBe(false)
    for (const c of calls) expect(typeof c.args?.settings?.gainDb).toBe('number')
    // Up, so louder than the -4 dB the harness seeds.
    const last = calls[calls.length - 1].args?.settings?.gainDb as number
    expect(last).toBeGreaterThan(-4)
  })

  test('a click that does not turn a knob sends nothing', async ({ page }) => {
    await page.locator('.sw-top .hwf-knob', { hasText: 'Pan' }).locator('canvas').click()
    await page.waitForTimeout(100)
    expect(await settingsCalls(page)).toHaveLength(0)
  })

  test('Escape closes it', async ({ page }) => {
    await page.keyboard.press('Escape')
    await expect(page.locator('.sw-win')).toHaveCount(0)
  })
})
