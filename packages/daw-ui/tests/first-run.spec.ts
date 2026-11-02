import { test, expect, type Page } from '@playwright/test'

/**
 * The first run, end to end in the whole app (index.html with the IPC mock
 * as the backend): one window from the start screen to a starter song, and
 * nothing stacked under or after it. The mock has two keyboards and a desk
 * plugged in (Akai, Launchkey, X-Touch).
 */
interface Call { cmd: string; args?: Record<string, unknown> }

const sent = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => ((window as unknown as { __hwMockCalls: Call[] }).__hwMockCalls).filter((x) => x.cmd === c).map((x) => x.args ?? null),
    cmd,
  )

const flow = (page: Page) => page.getByTestId('first-run')
const stepper = (page: Page) => page.getByTestId('first-run-stepper').first()

async function bootFresh(page: Page) {
  await page.goto('/')
  await expect(page.getByRole('button', { name: 'Set up and start a song' })).toBeVisible({ timeout: 20_000 })
}

test.describe('One first-run flow', () => {
  // The whole app boots, past the splash, in each of these.
  test.setTimeout(120_000)
  test('start screen, sound, controller, song: one window, ending in the Hardstyle starter', async ({ page }) => {
    await bootFresh(page)
    // One window: the start screen is the first step, with the stepper.
    await expect(stepper(page).locator('span').first()).toHaveAttribute('data-state', 'active')
    await expect(page.getByText('Start a new project or open a recent one.')).toHaveCount(0)
    await expect(page.getByText('Setup Wizard')).toHaveCount(0)

    await page.getByRole('button', { name: 'Set up and start a song' }).click()
    await expect(flow(page)).toHaveAttribute('data-step', 'sound')
    await expect(page.getByTestId('test-tone-strip')).toContainText('Test tone on Focusrite Scarlett 2i2')

    // Picking a device applies it and plays the tone on it.
    await page.getByLabel('Output device').selectOption('Speakers (Realtek HD Audio)')
    await expect(page.getByTestId('test-tone-strip')).toContainText('Test tone on Speakers (Realtek HD Audio)')
    expect((await sent(page, 'set_audio_config')).at(-1)).toMatchObject({ device: 'Speakers (Realtek HD Audio)' })

    await page.getByRole('button', { name: 'Continue' }).click()
    await expect(flow(page)).toHaveAttribute('data-step', 'controller')
    await expect(flow(page)).toContainText('Akai MPK Mini mk3')
    await expect(flow(page)).toContainText('Novation Launchkey 49')
    // The desk is not a keyboard: it is offered as a control surface later.
    await expect(flow(page)).not.toContainText('X-Touch')
    await expect.poll(() => sent(page, 'open_midi_input')).toEqual(
      expect.arrayContaining([{ portName: 'Akai MPK Mini mk3' }, { portName: 'Novation Launchkey 49' }]),
    )
    // The keys sound while the step shows (the dev build mounts it twice,
    // so the last word counts).
    await expect.poll(() => sent(page, 'set_check_voice').then((c) => c.at(-1))).toEqual({ enabled: true })
    // No velocity curve in the first run.
    await expect(flow(page)).not.toContainText('Soft')

    // A key played shows up.
    await page.evaluate(() => { (window as unknown as { __hwLastNote?: unknown }).__hwLastNote = { note: 48, velocity: 96 } })
    await expect(page.getByTestId('first-run-activity')).toContainText('C3 received, velocity 96')

    await page.getByRole('button', { name: 'Continue' }).click()
    await expect(flow(page)).toHaveAttribute('data-step', 'song')
    await expect.poll(() => sent(page, 'set_check_voice').then((c) => c.at(-1))).toEqual({ enabled: false })
    const open = page.getByRole('button', { name: 'Open the Hardstyle starter' })
    await expect(open).toBeVisible()
    await page.getByRole('button', { name: /^Rawstyle/ }).click()
    await expect(page.getByRole('button', { name: 'Open the Rawstyle starter' })).toBeVisible()
    await page.getByRole('button', { name: /^Hardstyle/ }).click()
    await open.click()

    await expect(flow(page)).toHaveCount(0)
    await expect.poll(() => sent(page, 'create_starter_song')).toEqual([{ genreId: 'hardstyle' }])
    // Nothing comes up after it: no start screen, no wizard, no New project.
    await expect(page.getByRole('button', { name: 'Set up and start a song' })).toHaveCount(0)
    await expect(page.getByText('Start a new project or open a recent one.')).toHaveCount(0)
    await expect(page.getByText('Choose a template to start from.')).toHaveCount(0)
    // The desk plugged in is offered once the first run is out of the way.
    await expect(page.getByTestId('notification-host')).toContainText('X-Touch plugged in: a control surface')
  })

  test('with no controller, Sound goes straight to Song and Controller is struck through', async ({ page }) => {
    await page.addInitScript(() => { (window as unknown as { __hwMidiPorts?: string[] }).__hwMidiPorts = [] })
    await bootFresh(page)
    await page.getByRole('button', { name: 'Set up and start a song' }).click()
    await expect(stepper(page).locator('span').nth(2)).toHaveAttribute('data-state', 'skipped')
    await page.getByRole('button', { name: 'Continue' }).click()
    await expect(flow(page)).toHaveAttribute('data-step', 'song')
    await expect(flow(page)).toContainText('Step 4 of 4')
    await page.getByRole('button', { name: 'Back' }).click()
    await expect(flow(page)).toHaveAttribute('data-step', 'sound')
  })

  test('it does not come back: not after a restart, not after New project', async ({ page }) => {
    await bootFresh(page)
    await page.getByRole('button', { name: 'Skip setup, open an empty project' }).click()
    await expect(page.getByRole('button', { name: 'Set up and start a song' })).toHaveCount(0)
    // A restart: a new session, so the start screen may show again.
    await page.evaluate(() => sessionStorage.clear())
    await page.reload()
    await page.waitForTimeout(6000)
    await expect(flow(page)).toHaveCount(0)
    await expect(page.getByRole('button', { name: 'Set up and start a song' })).toHaveCount(0)
    // Later launches show today's start screen, and no setup.
    await expect(page.getByText('Start a new project or open a recent one.')).toBeVisible()
    await page.getByRole('button', { name: 'New project', exact: true }).click()
    await expect(page.getByText('Choose a template to start from.')).toBeVisible()
    await expect(flow(page)).toHaveCount(0)
  })

  test('someone who finished the old setup wizard never sees it', async ({ page }) => {
    await page.addInitScript(() => {
      localStorage.setItem('hw-setup-wizard', JSON.stringify({
        state: { completedFirstRun: true, skippedAt: null, midiMasterEnabled: true, velocityCurves: {}, midiInputs: {} },
        version: 0,
      }))
    })
    await page.goto('/')
    await expect(page.getByText('Start a new project or open a recent one.')).toBeVisible({ timeout: 20_000 })
    await expect(flow(page)).toHaveCount(0)
    await expect(page.getByRole('button', { name: 'Set up and start a song' })).toHaveCount(0)
  })
})
