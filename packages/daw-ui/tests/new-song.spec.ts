import { test, expect, type Page } from '@playwright/test'

/**
 * A new song in the screenshot harness: Kick (the kick synth), Clap, Hat and
 * Snare as a new song opens with them. What a click sends is read back
 * from the IPC mock's call log.
 */
interface Call { cmd: string; args?: Record<string, unknown> }

const calls = (page: Page, cmd: string) =>
  page.evaluate((name) =>
    ((window as unknown as { __hwMockCalls: Call[] }).__hwMockCalls).filter((c) => c.cmd === name), cmd)

test.describe('A new song\'s channel rack', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/screenshot.html?panel=newsong')
    await expect(page.getByTestId('channel-name').first()).toBeVisible()
  })

  test('says what each channel plays', async ({ page }) => {
    await expect(page.getByTestId('channel-name')).toHaveText([/Kick\s*Kick synth/, /Clap\s*Clap 1/, /Hat\s*Hat 1/, /Snare\s*Snare 1/])
    // The harness rests the mouse on Clap.
    await expect(page.getByTestId('channel-name-tip')).toContainText('Clap 1.wav in Hardwave Sampler.')
    await expect(page.getByTestId('browser-drums')).toContainText('Clap 1.wav')
    await expect(page.getByTestId('browser-drums').getByText('in use')).toHaveCount(3)
  })

  test('a click on a step plays that channel with the song stopped', async ({ page }) => {
    const hat = page.locator('[data-channel-id="insert-003"]')
    await hat.locator('button[title="Click to add step"]').first().click()
    await expect.poll(async () => (await calls(page, 'inject_midi_event')).length).toBe(2)
    const route = await calls(page, 'set_keyboard_track')
    expect(route[0]?.args).toEqual({ trackId: 'insert-003' })
    const [on, off] = await calls(page, 'inject_midi_event')
    expect(on.args).toMatchObject({ event: { kind: 'note_on', note: 60 } })
    expect(off.args).toMatchObject({ event: { kind: 'note_off', note: 60 } })
  })
})
