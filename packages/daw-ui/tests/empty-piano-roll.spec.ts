import { test, expect, type Page } from '@playwright/test'

/**
 * The piano roll with nothing open in it, in a new song in the screenshot
 * harness (Kick, Clap, Hat and Snare). What a click sends is read back
 * from the IPC mock's call log.
 */
interface Call { cmd: string; args?: Record<string, unknown> }

const calls = (page: Page, cmd: string) =>
  page.evaluate((name) =>
    ((window as unknown as { __hwMockCalls: Call[] }).__hwMockCalls).filter((c) => c.cmd === name), cmd)

test.describe('The empty piano roll', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/screenshot.html?panel=emptyroll')
    await expect(page.getByTestId('empty-roll-picker')).toBeVisible()
  })

  test('offers the song\'s channels and the instruments to add', async ({ page }) => {
    await expect(page.getByText('Pick an instrument', { exact: true })).toBeVisible()
    await expect(page.getByText('Select a MIDI track or double-click a MIDI clip to edit notes.')).toHaveCount(0)
    const channels = page.getByTestId('roll-channel')
    await expect(channels).toHaveText([/Kick\s*Kick synth/, /Clap\s*Clap 1\.wav/, /Hat\s*Hat 1\.wav/, /Snare\s*Snare 1\.wav/])
    const add = page.getByTestId('roll-add')
    await expect(add.first()).toContainText('Hardwave 3xOSC')
    await expect(add).toHaveCount(6)
    await expect(page.getByTestId('roll-others')).toContainText('4 found')
  })

  test('opens on a channel of the song, with a clip made for it', async ({ page }) => {
    await page.getByTestId('roll-channel').filter({ hasText: 'Clap' }).click()
    await expect(page.getByTestId('empty-roll-picker')).toHaveCount(0)
    expect((await calls(page, 'create_midi_clip'))[0]?.args).toMatchObject({ trackId: 'insert-002' })
  })

  test('a click in the grid adds 3xOSC and puts the note there', async ({ page }) => {
    const grid = page.locator('canvas[role="application"]')
    const box = (await grid.boundingBox())!
    // Away from the card in the middle.
    await page.mouse.click(box.x + 40, box.y + 40)
    await expect(page.getByTestId('empty-roll-picker')).toHaveCount(0)
    expect((await calls(page, 'add_midi_track'))[0]?.args).toEqual({ name: 'Hardwave 3xOSC' })
    expect((await calls(page, 'add_plugin_to_track'))[0]?.args).toMatchObject({ pluginId: 'hardwave.native.tripleosc' })
    const note = (await calls(page, 'add_midi_note'))[0]?.args
    expect(note).toMatchObject({ startTick: expect.any(Number), pitch: expect.any(Number) })
    expect(Number(note?.pitch)).toBeGreaterThan(0)
  })
})
