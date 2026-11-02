import { test, expect, type Page } from '@playwright/test'

/**
 * Controllers plugged in later, in the whole app (index.html with the IPC
 * mock as the backend). The engine's notice is played by the mock:
 * `__hwMockEmit('midi-input-appeared', { port, kind })`. Setup was finished
 * before, with nothing plugged in, so every input the mock lists is new.
 */
interface Call { cmd: string; args?: Record<string, unknown> }

const sent = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => ((window as unknown as { __hwMockCalls: Call[] }).__hwMockCalls).filter((x) => x.cmd === c).map((x) => x.args ?? null),
    cmd,
  )

const toasts = (page: Page) => page.getByTestId('notification-host')

/** Plug an input in: the system lists it, and the engine says so. */
const plugIn = (page: Page, port: string, kind: 'keyboard' | 'desk' | 'virtual') =>
  page.evaluate(({ port, kind }) => {
    const w = window as unknown as { __hwMockEmit: (e: string, p: unknown) => void; __hwMidiPorts?: string[] }
    w.__hwMidiPorts = [...(w.__hwMidiPorts ?? []), port]
    w.__hwMockEmit('midi-input-appeared', { port, kind })
  }, { port, kind })

const remembered = (page: Page) =>
  page.evaluate(() => JSON.parse(localStorage.getItem('hw-setup-wizard') ?? '{}').state?.midiInputs ?? {})

async function boot(page: Page, atLaunch: string[] = [], settle = true) {
  await page.addInitScript((ports) => {
    // What is plugged in at launch (nothing, unless a test says); setup
    // finished long ago.
    (window as unknown as { __hwMidiPorts?: string[] }).__hwMidiPorts = ports
    if (!localStorage.getItem('hw-setup-wizard')) {
      localStorage.setItem('hw-setup-wizard', JSON.stringify({
        state: { completedFirstRun: true, skippedAt: null, midiMasterEnabled: true, velocityCurves: {}, midiInputs: {} },
        version: 0,
      }))
    }
    localStorage.setItem('hardwave.daw.welcomeSkip', '1')
  }, atLaunch)
  await page.goto('/')
  if (settle) await page.waitForTimeout(5000)
}

test.describe('Controllers plugged in later', () => {
  test('a new controller turns on by itself, with a message and Turn off', async ({ page }) => {
    await boot(page)
    await plugIn(page, 'Akai MPK Mini mk3', 'keyboard')
    await expect(toasts(page)).toContainText('Akai MPK Mini mk3 plugged in and turned on')
    await expect(toasts(page)).toContainText('Its keys play the selected channel (Lead).')
    expect(await sent(page, 'open_midi_input')).toEqual([{ portName: 'Akai MPK Mini mk3' }])
    expect(await remembered(page)).toEqual({ 'Akai MPK Mini mk3': true })
  })

  test('Turn off is remembered for that device, across a restart', async ({ page }) => {
    await boot(page)
    await plugIn(page, 'Akai MPK Mini mk3', 'keyboard')
    await toasts(page).getByRole('button', { name: 'Turn off' }).click()
    await expect.poll(() => sent(page, 'close_midi_input')).toEqual([{ portName: 'Akai MPK Mini mk3' }])
    expect(await remembered(page)).toEqual({ 'Akai MPK Mini mk3': false })

    await page.reload()
    await page.waitForTimeout(5000)
    await plugIn(page, 'Akai MPK Mini mk3', 'keyboard')
    await page.waitForTimeout(500)
    expect(await sent(page, 'open_midi_input')).toEqual([])
    await expect(toasts(page)).not.toContainText('Akai MPK Mini mk3')
  })

  test('MIDI settings opens the MIDI tab, where it is Connected', async ({ page }) => {
    await boot(page)
    await plugIn(page, 'Akai MPK Mini mk3', 'keyboard')
    await toasts(page).getByRole('button', { name: 'MIDI settings' }).click()
    await expect(page.getByRole('tab', { name: 'MIDI' })).toHaveAttribute('aria-selected', 'true')
    await expect(page.getByLabel('Velocity curve for Akai MPK Mini mk3')).toHaveValue('linear')
    await expect(page.getByRole('button', { name: 'Connected' })).toBeVisible()
  })

  test('a control surface is offered as one, not opened as a keyboard', async ({ page }) => {
    await boot(page)
    await plugIn(page, 'Behringer X-Touch', 'desk')
    await expect(toasts(page)).toContainText('Behringer X-Touch plugged in: a control surface')
    expect(await sent(page, 'open_midi_input')).toEqual([])
    await toasts(page).getByRole('button', { name: 'Use as control surface' }).click()
    await expect.poll(() => sent(page, 'set_control_surface_enabled')).toEqual([{ enabled: true }])
    expect(await sent(page, 'set_control_surface_input')).toEqual([{ input: 'Behringer X-Touch' }])
    expect(await sent(page, 'open_midi_input')).toEqual([{ portName: 'Behringer X-Touch' }])
  })

  test('a virtual port is left alone', async ({ page }) => {
    await boot(page)
    await plugIn(page, 'loopMIDI Port', 'virtual')
    await page.waitForTimeout(500)
    expect(await sent(page, 'open_midi_input')).toEqual([])
    await expect(toasts(page)).not.toContainText('loopMIDI')
  })

  test('one plugged in while the DAW was closed gets the same message at launch', async ({ page }) => {
    // Not waiting for the app to settle: the message goes after six seconds.
    await boot(page, ['Arturia KeyStep 37'], false)
    await expect(toasts(page)).toContainText('Arturia KeyStep 37 plugged in and turned on')
  })
})
