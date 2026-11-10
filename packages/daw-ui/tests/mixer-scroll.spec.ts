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
    const top = Math.max(...state.tracks.map((t) => t.mixerNumber ?? 0))
    const extra = Array.from({ length: 80 }, (_, i) => ({ ...base, id: 'extra-' + i, name: 'Ch ' + (i + 1), clips: [], mixerNumber: top + 1 + i }))
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

// The same with FL's numbers: inserts 1 to 20, then empty numbers drawn up
// to 125. Strips 69 to 79 went blank in the founder's song; with the empty
// numbers drawn, that far along is all empty strips, and they must show.
test('the mixer scrolled past the inserts shows the empty numbers that are there', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 720 })
  await page.goto('/screenshot.html?panel=mixer&song=fl')
  await page.waitForSelector('.mx-strips-wrap')
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
        return r.right > view.left + 1 && r.left < view.right - 1 && r.height > 100
      })
      .map((s) => ({ n: Number(s.getAttribute('data-idx')), empty: s.classList.contains('empty') }))
      .sort((a, b) => a.n - b.n)
  })
  const scrolled = await wrap.evaluate((el) => el.scrollLeft)
  expect(seen.length).toBeGreaterThanOrEqual(Math.floor(box.width / 84))
  expect(seen[0].n).toBe(Math.floor(scrolled / 84) + 1)
  expect(seen[0].n).toBeGreaterThan(60)
  expect(seen.map((s) => s.n)).toContain(75)
  expect(seen.every((s) => s.empty)).toBe(true)
  // Numbers follow on with no gap.
  seen.forEach((s, i) => expect(s.n).toBe(seen[0].n + i))
})

// FL's inserts in number order with their names; channels are not strips.
test("the mixer shows FL's inserts by number and name, and the channels leave it", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 720 })
  await page.goto('/screenshot.html?panel=mixer&song=fl')
  await page.waitForSelector('.mx-strips-wrap .mx-strip')
  const first = await page.evaluate(() =>
    [...document.querySelectorAll('.mx-strips-wrap .mx-strip[data-idx]')]
      .map((s) => ({ n: Number(s.getAttribute('data-idx')), name: s.querySelector('.mx-s-name')?.textContent ?? '' }))
      .sort((a, b) => a.n - b.n)
      .slice(0, 8),
  )
  expect(first.map((s) => s.n)).toEqual([1, 2, 3, 4, 5, 6, 7, 8])
  expect(first[3].name).toBe('Insert 4')
  expect(first[6].name).toBe('THM - Pad 11 - A# - 127 BPM')
  expect(await page.locator('.mx-strip[data-track-id^="fl-ch-"]').count()).toBe(0)
})
