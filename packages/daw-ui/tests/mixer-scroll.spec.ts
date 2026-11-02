import { test, expect } from '@playwright/test'

// The mixer's strips row moved twice as far as the strips it had mounted:
// a transform on the row and the viewport's own scroll both took the
// offset. Scrolled to channel 70 of 80 the strips area was empty.
test('the mixer scrolled far along shows the strips that are there', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 720 })
  await page.goto('/screenshot.html?panel=mixer')
  await page.waitForSelector('.mx-strips-wrap')
  // Eighty more channels in the harness's store. A string, so the test
  // runner leaves the module import for the page to resolve.
  await page.evaluate(`(async () => {
    const { useTrackStore } = await import('/src/stores/trackStore.ts')
    const state = useTrackStore.getState()
    const base = state.tracks.find((t) => t.kind === 'Audio')
    const extra = Array.from({ length: 80 }, (_, i) => ({ ...base, id: 'extra-' + i, name: 'Ch ' + (i + 1), clips: [] }))
    const tracks = [...state.tracks, ...extra]
    useTrackStore.setState({ tracks, tracksById: Object.fromEntries(tracks.map((t) => [t.id, t])) })
  })()`)
  const wrap = page.locator('.mx-strips-wrap')
  const box = (await wrap.boundingBox())!
  await page.mouse.move(box.x + box.width / 2, box.y + 30)
  for (let i = 0; i < 26; i++) {
    await page.mouse.wheel(0, 220)
    await page.waitForTimeout(40)
  }
  await page.waitForTimeout(1500)

  const seen = await page.evaluate(() => {
    const el = document.querySelector('.mx-strips-wrap')!
    const view = el.getBoundingClientRect()
    return [...el.querySelectorAll('.mx-strip[data-idx]')]
      .filter((s) => {
        const r = s.getBoundingClientRect()
        return r.right > view.left + 1 && r.left < view.right - 1
      })
      .map((s) => Number(s.getAttribute('data-idx')))
      .sort((a, b) => a - b)
  })
  const scrolled = await wrap.evaluate((el) => el.scrollLeft)
  // Every strip slot on screen has its strip, and they are the ones at
  // this scroll position, 84 px each.
  expect(seen.length).toBeGreaterThanOrEqual(Math.floor(box.width / 84))
  expect(seen[0]).toBe(Math.floor(scrolled / 84) + 1)
  expect(seen[0]).toBeGreaterThan(60)
})
