import { test, expect, type Page } from '@playwright/test'

/**
 * The control surface row in Settings, MIDI tab, over the screenshot
 * harness: the protocol and desk input pickers beside On, and the caption
 * that follows the protocol. The harness opens a keyboard (Launchkey 49)
 * and a desk (X-Touch); what a picker sends is read back from the IPC
 * mock's call log.
 */
interface Call { cmd: string; args?: Record<string, unknown> }

async function boot(page: Page) {
  await page.goto('/screenshot.html?panel=settings&tab=midi')
  await expect(page.getByLabel('Desk protocol')).toBeVisible()
}

const sent = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => ((window as unknown as { __hwMockCalls: Call[] }).__hwMockCalls).filter((x) => x.cmd === c).map((x) => x.args),
    cmd,
  )

const optionTexts = (page: Page) =>
  page.getByLabel('Desk input').locator('option').allTextContents()

test.describe('Control surface pickers', () => {
  test.beforeEach(async ({ page }) => boot(page))

  test('the protocol picker switches the desk and the caption with it', async ({ page }) => {
    const caption = page.getByTestId('surface-caption')
    await expect(page.getByLabel('Desk protocol')).toHaveValue('mackie')
    await expect(caption).toContainText('Mackie Control (MCU) and the desks that copy it')
    await expect(caption).not.toContainText('not supported')

    await page.getByLabel('Desk protocol').selectOption('hui')
    await expect(caption).toContainText('the scribble strips show four letters of each name')
    expect(await sent(page, 'set_control_surface_protocol')).toEqual([{ protocol: 'hui' }])
  })

  test('the desk is found by its name, and another input can be picked', async ({ page }) => {
    const picker = page.getByLabel('Desk input')
    await expect(picker).toHaveValue('X-Touch')
    expect(await optionTexts(page)).toEqual([
      'Desk: X-Touch (found)',
      'Desk: Novation Launchkey 49',
      'Desk: every MIDI input',
    ])

    await picker.selectOption('')
    await expect(picker).toHaveValue('')
    await picker.selectOption('X-Touch')
    await expect(picker).toHaveValue('X-Touch')
    // Picked by hand now, so no longer "found".
    await expect.poll(() => optionTexts(page)).toContain('Desk: X-Touch')
    expect(await sent(page, 'set_control_surface_input')).toEqual([{ input: null }, { input: 'X-Touch' }])
  })

  test('the pickers sit beside On and go with it', async ({ page }) => {
    const row = page.locator('[data-setting-label^="control surface"]')
    await row.getByRole('button', { name: 'On' }).click()
    await expect(page.getByLabel('Desk protocol')).toHaveCount(0)
    await expect(page.getByLabel('Desk input')).toHaveCount(0)
    expect(await sent(page, 'set_control_surface_enabled')).toEqual([{ enabled: false }])
  })
})
