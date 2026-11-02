import { test, expect, type Page } from '@playwright/test'

/**
 * The test tone after Apply in Settings > Audio, over the screenshot
 * harness. The IPC mock plays the backend: it applies a device, plays the
 * tone (0.4 s left, then 0.4 s right) and fails to open the device when a
 * spec sets `__hwAudioFails`. What the window sends is read back from the
 * mock's call log.
 */
interface Call { cmd: string; args?: Record<string, unknown> }

const sent = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => ((window as unknown as { __hwMockCalls: Call[] }).__hwMockCalls).filter((x) => x.cmd === c).map((x) => x.args ?? null),
    cmd,
  )

async function boot(page: Page, query = '') {
  await page.goto(`/screenshot.html?panel=settings&tab=audio${query}`)
  await expect(page.locator('[data-setting-label^="output device"] select')).toBeVisible()
}

const strip = (page: Page) => page.getByTestId('test-tone-strip')

test.describe('Test tone after Apply', () => {
  test('Apply plays the tone on the device just applied, and names it', async ({ page }) => {
    await boot(page)
    await expect(strip(page)).toHaveCount(0)
    await page.locator('[data-setting-label^="output device"] select').selectOption('Speakers (Realtek HD Audio)')
    await page.getByRole('button', { name: 'Apply', exact: true }).click()

    await expect(strip(page)).toContainText('Test tone on Speakers (Realtek HD Audio)')
    await expect(strip(page)).toContainText(/Outputs 1\/2 · left, then right · applied \d\d:\d\d/)
    const order = await page.evaluate(() =>
      ((window as unknown as { __hwMockCalls: Call[] }).__hwMockCalls)
        .map((c) => c.cmd).filter((c) => c === 'set_audio_config' || c === 'play_test_tone'))
    expect(order).toEqual(['set_audio_config', 'play_test_tone'])
    // The meters move while it plays: left first.
    await expect.poll(async () =>
      page.getByTestId('test-tone-meter-L').locator('i').evaluate((el) => parseFloat((el as HTMLElement).style.width)),
    ).toBeGreaterThan(30)
  })

  test('Test sound plays it again at any time', async ({ page }) => {
    await boot(page)
    await page.getByRole('button', { name: 'Test sound' }).click()
    await expect(strip(page)).toContainText('Test tone on Focusrite Scarlett 2i2')
    await strip(page).getByRole('button', { name: 'Play again' }).click()
    await expect.poll(() => sent(page, 'play_test_tone').then((c) => c.length)).toBe(2)
  })

  test('I hear nothing lists what to check', async ({ page }) => {
    await boot(page)
    await page.getByRole('button', { name: 'Test sound' }).click()
    await strip(page).getByRole('button', { name: 'I hear nothing' }).click()
    const help = page.getByTestId('test-tone-help')
    await expect(help).toContainText("interface's volume knob")
    await expect(help).toContainText('another output device')
    await expect(help).toContainText('exclusive mode')
  })

  test('a device that does not open says why, with Retry', async ({ page }) => {
    await boot(page)
    await page.evaluate(() => { (window as unknown as { __hwAudioFails?: string }).__hwAudioFails = 'Another program is using the sound card' })
    await page.locator('[data-setting-label^="buffer size"] select').selectOption('256')
    await page.getByRole('button', { name: 'Apply', exact: true }).click()
    await expect(strip(page)).toContainText('The sound card did not open')
    await expect(strip(page)).toContainText('Another program is using the sound card')
    expect(await sent(page, 'play_test_tone')).toEqual([])

    await page.evaluate(() => { delete (window as unknown as { __hwAudioFails?: string }).__hwAudioFails })
    await strip(page).getByRole('button', { name: 'Retry' }).click()
    await expect(strip(page)).toContainText('Test tone on Focusrite Scarlett 2i2')
  })

  test('a new output pair plays the tone on it', async ({ page }) => {
    await page.addInitScript(() => { (window as unknown as { __hwOutputChannels?: number }).__hwOutputChannels = 4 })
    await boot(page)
    await page.locator('[data-setting-label^="output pair"] select').selectOption('2')
    await expect(strip(page)).toContainText('Test tone on Focusrite Scarlett 2i2')
    await expect(strip(page)).toContainText('Outputs 3/4')
    expect(await sent(page, 'set_channel_offsets')).toEqual([{ input: 0, output: 2 }])
  })

  test('while a song plays, Apply does not play over it; Test sound still does', async ({ page }) => {
    await boot(page, '&playing=1')
    await page.locator('[data-setting-label^="buffer size"] select').selectOption('256')
    await page.getByRole('button', { name: 'Apply', exact: true }).click()
    await expect.poll(() => sent(page, 'set_audio_config').then((c) => c.length)).toBe(1)
    expect(await sent(page, 'play_test_tone')).toEqual([])
    await page.getByRole('button', { name: 'Test sound' }).click()
    await expect.poll(() => sent(page, 'play_test_tone').then((c) => c.length)).toBe(1)
  })
})
