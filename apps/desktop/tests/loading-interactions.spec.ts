import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

test.beforeEach(async ({ page }) => seedLibrary(page))

test('partially refreshed pages can overlap without breaking the poster wall', async ({ page }) => {
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  await overrideApi(page, `
    const list = api.libraryList;
    window.overlapRefresh = false;
    api.libraryList = async q => {
      const result = await list(q);
      if (window.overlapRefresh && q.offset === 0) {
        const next = await list({ ...q, offset: 120 });
        result.items[119] = next.items[0];
        window.pageZeroUpdated = true;
      }
      if (window.overlapRefresh && q.offset === 120) await new Promise(r => window.releasePage = r);
      return result;
    };
  `)
  await page.goto('/')
  await expect(page.locator('button.pcard').first()).toBeVisible()
  await page.locator('.scroller').evaluate(scroller => {
    const cards = [...scroller.querySelectorAll('button.pcard')]
    const top = cards[0].getBoundingClientRect().top
    const cols = cards.filter(card => Math.abs(card.getBoundingClientRect().top - top) < 1).length
    const rowHeight = cards[cols].getBoundingClientRect().top - top
    scroller.scrollTop = rowHeight * Math.floor(120 / cols)
  })
  await expect(page.locator('.scroller')).toHaveAttribute('aria-busy', 'false')
  await expect(page.locator('.scroller .skel')).toHaveCount(0)
  await page.evaluate(async () => {
    ;(window as any).overlapRefresh = true
    const { app } = await import('/src/lib/state.svelte.ts' as string)
    app.libraryVersion++
  })
  await expect.poll(() => page.evaluate(() => (window as any).pageZeroUpdated)).toBe(true)
  await page.evaluate(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))))
  expect(errors).toEqual([])
  await expect.poll(() => page.evaluate(() => typeof (window as any).releasePage)).toBe('function')
  await page.evaluate(() => (window as any).releasePage())
  await expect(page.locator('.scroller')).toHaveAttribute('aria-busy', 'false')
})

test('virtual rows keep the same geometry when the buffered window moves', async ({ page }) => {
  await page.goto('/')
  await expect(page.locator('button.pcard').first()).toBeVisible()
  const initial = await page.locator('.scroller').evaluate(scroller => {
    const cards = [...scroller.querySelectorAll<HTMLButtonElement>('button.pcard')]
    const top = cards[0].getBoundingClientRect().top
    const cols = cards.filter(card => Math.abs(card.getBoundingClientRect().top - top) < 1).length
    const rowHeight = cards[cols].getBoundingClientRect().top - top
    const target = cards[cols * 3]
    return { first: cards[0].getAttribute('aria-label')!, name: target.getAttribute('aria-label')!, y: target.getBoundingClientRect().top, scroll: rowHeight * 3.25 }
  })
  const actualScroll = await page.locator('.scroller').evaluate((scroller, scroll) => { scroller.scrollTop = scroll; return scroller.scrollTop }, initial.scroll)
  await expect(page.getByRole('button', { name: initial.first, exact: true })).toHaveCount(0)
  await expect.poll(() => page.locator('.scroller').evaluate(scroller => scroller.scrollTop)).toBe(actualScroll)
  const target = page.getByRole('button', { name: initial.name, exact: true })
  await expect.poll(async () => (await target.boundingBox())!.y).toBeCloseTo(initial.y - actualScroll, 0)
})

test('library reserves poster space during loading and ignores late search results', async ({ page }) => {
  await overrideApi(page, `
    const list = api.libraryList;
    window.releaseSearch = null;
    api.libraryList = async q => {
      if (!q.text) await new Promise(r => setTimeout(r, 900));
      const result = await list(q);
      if (q.text === 'late') await new Promise(r => window.releaseSearch = r);
      return result;
    };
  `)
  await page.goto('/')
  await expect(page.locator('.scroller .skel').first()).toBeVisible()
  await expect(page.locator('button.pcard').first()).toBeVisible()
  const search = page.getByRole('searchbox')
  await search.fill('late')
  await expect.poll(() => page.evaluate(() => typeof (window as any).releaseSearch)).toBe('function')
  await expect(page.locator('.scroller')).toHaveAttribute('aria-busy', 'true')
  await search.fill('no-match-new')
  await expect(page.getByText('没有匹配的影片', { exact: true })).toBeVisible()
  await page.evaluate(() => (window as any).releaseSearch())
  await expect(page.locator('.scroller')).toHaveAttribute('aria-busy', 'false')
  await expect(page.getByText('没有匹配的影片', { exact: true })).toBeVisible()
})

test('pending read failure stays distinct from empty and can retry', async ({ page }) => {
  await overrideApi(page, `
    const pending = api.pendingList;
    window.pendingFails = true;
    api.pendingList = async () => {
      if (window.pendingFails) throw new Error('source unavailable');
      return pending();
    };
  `)
  await page.goto('/')
  await page.getByRole('tab', { name: '待整理' }).click()
  await expect(page.locator('.pending').getByRole('alert')).toContainText('source unavailable')
  await expect(page.getByText('还没有待整理的文件夹', { exact: true })).toHaveCount(0)
  await page.evaluate(() => (window as any).pendingFails = false)
  await page.locator('.pending').getByRole('button', { name: '重新读取' }).click()
  await expect(page.getByText('还没有待整理的文件夹', { exact: true })).toBeVisible()
})

test('refresh keeps posters visible while exposing busy state', async ({ page }) => {
  await overrideApi(page, `
    const list = api.libraryList;
    window.holdRefresh = false;
    api.libraryList = async q => {
      if (window.holdRefresh) await new Promise(r => window.releaseRefresh = r);
      return list(q);
    };
  `)
  await page.goto('/')
  await expect(page.locator('button.pcard').first()).toBeVisible()
  await page.evaluate(async () => {
    ;(window as any).holdRefresh = true
    const { app } = await import('/src/lib/state.svelte.ts' as string)
    app.libraryVersion++
  })
  await expect(page.locator('.scroller')).toHaveAttribute('aria-busy', 'true')
  await expect(page.locator('button.pcard').first()).toBeVisible()
  await page.evaluate(() => (window as any).releaseRefresh())
  await expect(page.locator('.scroller')).toHaveAttribute('aria-busy', 'false')
})
