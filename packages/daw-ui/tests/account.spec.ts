import { test, expect, type Page } from '@playwright/test'

/**
 * The account at the top right (components/account/AccountMenu.tsx) over the
 * screenshot harness, whose IPC mock plays the backend: it signs in with any
 * password but "wrong". What the window sends is read back from the mock's
 * call log, to prove the window asks the backend and never the site itself.
 */
interface Call { cmd: string; args?: Record<string, unknown> }

const calls = (page: Page) =>
  page.evaluate(() => (window as unknown as { __hwMockCalls: Call[] }).__hwMockCalls)

async function boot(page: Page, account: string) {
  await page.goto(`/screenshot.html?panel=account&account=${account}`)
  await expect(page.locator('.hw-acct')).toBeVisible()
}

test.describe('Account in the DAW', () => {
  test('signed out: a red Sign in that opens the sign-in panel', async ({ page }) => {
    await boot(page, 'signedout')
    const chip = page.locator('.hw-acct')
    await expect(chip).toHaveText('Sign in')
    await expect(chip).toHaveClass(/\bout\b/)
    await chip.click()
    const panel = page.locator('.hw-acct-pop')
    await expect(panel.locator('h3')).toHaveText('Sign in to Hardwave')
    await expect(panel.getByLabel('E-mail')).toBeFocused()
    await expect(panel.getByRole('button', { name: 'Make an account (opens the site)' })).toBeVisible()
    await panel.getByRole('button', { name: 'Why sign in?' }).click()
    await expect(panel).toContainText('Pro adds what runs on our servers')
  })

  test('signing in goes through the backend, then shows the name and PRO', async ({ page }) => {
    await boot(page, 'signedout')
    await page.locator('.hw-acct').click()
    const panel = page.locator('.hw-acct-pop')
    await panel.getByLabel('E-mail').fill('dishairano@hardwavestudios.com')
    await panel.getByLabel('Password').fill('hunter2')
    await panel.getByLabel('Password').press('Enter')
    const chip = page.locator('.hw-acct')
    await expect(chip).toContainText('Dishairano')
    await expect(chip.locator('.hw-acct-tag')).toHaveText('PRO')
    // The card replaces the form, and the password is gone from the page.
    await expect(panel.locator('.hw-acct-kv').first()).toContainText('Pro')
    await expect(page.locator('input[type=password]')).toHaveCount(0)
    const signIns = (await calls(page)).filter(c => c.cmd === 'account_sign_in')
    expect(signIns).toEqual([{ cmd: 'account_sign_in', args: { email: 'dishairano@hardwavestudios.com', password: 'hunter2' } }])
  })

  test('a refused sign-in says why and keeps the form', async ({ page }) => {
    await boot(page, 'signedout')
    await page.locator('.hw-acct').click()
    const panel = page.locator('.hw-acct-pop')
    await panel.getByLabel('E-mail').fill('dishairano@hardwavestudios.com')
    await panel.getByLabel('Password').fill('wrong')
    await panel.getByRole('button', { name: 'Sign in', exact: true }).click()
    await expect(panel.getByRole('alert')).toHaveText('That e-mail address and password do not match an account.')
    await expect(page.locator('.hw-acct')).toHaveText('Sign in')
    // The address stays to correct; the password does not.
    await expect(panel.getByLabel('E-mail')).toHaveValue('dishairano@hardwavestudios.com')
    await expect(panel.getByLabel('Password')).toHaveValue('')
  })

  test('signed in with Pro: the card shows the plan and signs out', async ({ page }) => {
    await boot(page, 'pro')
    const chip = page.locator('.hw-acct')
    await expect(chip).toContainText('Dishairano')
    await chip.click()
    const card = page.locator('.hw-acct-pop')
    await expect(card).toContainText('dis…@hardwavestudios.com')
    await expect(card.locator('.hw-acct-kv', { hasText: 'Plan' })).toContainText('Pro')
    await expect(card.locator('.hw-acct-kv', { hasText: 'Pro plug-ins' })).toContainText('all unlocked')
    await expect(card.locator('.hw-acct-kv', { hasText: 'Signed in until' })).toContainText('renews by itself')
    await card.getByRole('button', { name: 'Manage your account (opens the site)' }).click()
    expect((await calls(page)).filter(c => c.cmd === 'account_open_page')).toEqual([
      { cmd: 'account_open_page', args: { page: 'account' } },
    ])
    await card.getByRole('button', { name: 'Sign out' }).click()
    await expect(chip).toHaveText('Sign in')
    await expect(page.locator('.hw-acct-pop')).toHaveCount(0)
  })

  test('a free account says FREE and the plan only, with no upgrade button', async ({ page }) => {
    await boot(page, 'free')
    const chip = page.locator('.hw-acct')
    await expect(chip.locator('.hw-acct-tag')).toHaveText('FREE')
    await chip.click()
    const card = page.locator('.hw-acct-pop')
    await expect(card.locator('.hw-acct-kv', { hasText: 'Plan' })).toContainText('Free')
    await expect(card.locator('.hw-acct-kv', { hasText: 'Pro plug-ins' })).toContainText('free in this DAW')
    await expect(card.getByRole('button')).toHaveText([
      'Manage your account (opens the site)',
      'Sign out',
    ])
  })

  test('a sign-in that ran out asks to sign in again', async ({ page }) => {
    await boot(page, 'ended')
    await page.locator('.hw-acct').click()
    await expect(page.locator('.hw-acct-pop')).toContainText('Sign in again')
  })

  test('coming back to the window checks the account again', async ({ page }) => {
    await boot(page, 'signedout')
    await expect(page.locator('.hw-acct')).toHaveText('Sign in')
    // Signed in in the Suite meanwhile, then back to the DAW.
    await page.evaluate(() => {
      (globalThis as { __hwAccount?: unknown }).__hwAccount = {
        state: 'signedIn', name: 'Dishairano', email: 'dis…@hardwavestudios.com',
        plan: { kind: 'free' },
      }
    })
    await page.waitForTimeout(10_100)
    await page.evaluate(() => window.dispatchEvent(new Event('focus')))
    await expect(page.locator('.hw-acct')).toContainText('Dishairano')
  })

  test('Escape and a click outside close the panel', async ({ page }) => {
    await boot(page, 'pro')
    await page.locator('.hw-acct').click()
    await expect(page.locator('.hw-acct-pop')).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(page.locator('.hw-acct-pop')).toHaveCount(0)
    await page.locator('.hw-acct').click()
    await page.mouse.click(400, 400)
    await expect(page.locator('.hw-acct-pop')).toHaveCount(0)
  })
})
