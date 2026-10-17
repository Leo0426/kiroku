import { expect, test, type Page } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

test.use({ reducedMotion: 'no-preference', viewport: { width: 1100, height: 760 } })

async function finishImport(page: Page) {
  await seedLibrary(page)
  await overrideApi(page, `
    const getSettings = api.getSettings;
    api.getSettings = async () => ({ ...(await getSettings()), source_dirs: ['/mock/source'] });
    const scrapeStart = api.scrapeStart;
    api.scrapeStart = async (ids) => scrapeStart(ids.filter(id => id === 3));
    window.playCalls = [];
    api.play = async (code) => { window.playCalls.push(code); };
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await page.getByRole('button', { name: /查找影片信息/ }).click()
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  await page.getByRole('button', { name: /确认整理/ }).click()
  await expect(page.getByRole('button', { name: '查看媒体库', exact: true })).toBeVisible()
}

test('viewing the library after import never opens a movie and navigation stays usable', async ({ page }) => {
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  await finishImport(page)
  await page.getByRole('button', { name: '查看媒体库', exact: true }).click()
  await expect(page.getByRole('heading', { name: '媒体库', exact: true })).toBeVisible()
  await expect(page.getByLabel('影片详情', { exact: true })).toHaveCount(0)
  for (const name of ['历史', '设置', '导入整理', '媒体库']) {
    await page.getByRole('navigation', { name: '主导航' }).getByRole('button', { name, exact: true }).click()
    await expect(page.getByRole('heading', { name, exact: true })).toBeVisible()
  }
  expect(await page.evaluate(() => (window as unknown as { playCalls: string[] }).playCalls)).toEqual([])
  expect(errors).toEqual([])
})

test('a rapid double click on view library does not fall through to a poster', async ({ page }) => {
  await finishImport(page)
  const box = await page.getByRole('button', { name: '查看媒体库', exact: true }).boundingBox()
  if (!box) throw new Error('completion button is missing')
  const x = box.x + box.width / 2
  const y = box.y + box.height / 2
  await page.mouse.click(x, y)
  await expect(page.locator('button.pcard').first()).toBeVisible()
  // Ensure this layout actually exercises click-through, rather than a gap between cards.
  expect(await page.evaluate(({ x, y }) => !!document.elementFromPoint(x, y)?.closest('button.pcard'), { x, y })).toBe(true)
  // A second physical click lands on a poster at this window width.
  await page.mouse.click(x, y)
  await expect(page.getByRole('heading', { name: '媒体库', exact: true })).toBeVisible()
  await expect(page.getByLabel('影片详情', { exact: true })).toHaveCount(0)
  // Once the repeat-click window has passed, the same poster is usable normally.
  await page.waitForTimeout(550)
  await page.mouse.click(x, y)
  await expect(page.getByLabel('影片详情', { exact: true })).toBeVisible()
  await page.getByRole('navigation', { name: '主导航' }).getByRole('button', { name: '历史', exact: true }).click()
  await expect(page.getByRole('heading', { name: '历史', exact: true })).toBeVisible()
  expect(await page.evaluate(() => (window as unknown as { playCalls: string[] }).playCalls)).toEqual([])
})

test('movie details leave the main navigation accessible', async ({ page }) => {
  await seedLibrary(page)
  await page.goto('/')
  await page.locator('button.pcard').first().click()
  await expect(page.getByLabel('影片详情', { exact: true })).toBeVisible()
  const history = page.getByRole('navigation', { name: '主导航' }).getByRole('button', { name: '历史', exact: true })
  // Use a physical click: locator.click would retry until the covering scrim disappears.
  const box = await history.boundingBox()
  if (!box) throw new Error('navigation button is missing')
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2)
  await expect(page.getByRole('heading', { name: '历史', exact: true })).toBeVisible()
  await expect(page.getByLabel('影片详情', { exact: true })).toHaveCount(0)
})

test('keyboard activation still opens a deliberately selected movie', async ({ page }) => {
  await finishImport(page)
  await page.getByRole('button', { name: '查看媒体库', exact: true }).focus()
  await page.keyboard.press('Enter')
  const card = page.locator('button.pcard').first()
  await card.focus()
  await page.keyboard.press('Enter')
  await expect(page.getByLabel('影片详情', { exact: true })).toBeVisible()
  expect(await page.evaluate(() => (window as unknown as { playCalls: string[] }).playCalls)).toEqual([])
  await page.getByLabel('影片详情', { exact: true }).getByRole('button', { name: '播放', exact: true }).first().click()
  await expect.poll(() => page.evaluate(() => (window as unknown as { playCalls: string[] }).playCalls.length)).toBe(1)
})
