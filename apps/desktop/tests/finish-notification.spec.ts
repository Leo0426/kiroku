import { expect, test, type Page } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

async function setup(page: Page, focused: boolean, enabled = true) {
  await seedLibrary(page)
  // 设置关闭要写进 mock 自己的设置,mock 的 notifyFinish 读的是它
  if (!enabled) await page.addInitScript(() => {
    localStorage.setItem('kiroku-mock-settings-v2', JSON.stringify({ library_root: '/mock/library', notify_on_finish: false }))
  })
  await page.addInitScript((f) => { document.hasFocus = () => f }, focused)
  await overrideApi(page, `
    const getSettings = api.getSettings;
    api.getSettings = async () => ({ ...(await getSettings()), source_dirs: ['/mock/source'] });
    const scrapeStart = api.scrapeStart;
    api.scrapeStart = async (ids) => scrapeStart(ids.slice(0, 2));
    window.notices = [];
    const notifyFinish = api.notifyFinish;
    api.notifyFinish = async (title, body) => { const sent = await notifyFinish(title, body); window.notices.push({ title, body, sent }); return sent };
  `)
  await page.goto('/')
}

async function organize(page: Page) {
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await page.getByRole('button', { name: /查找影片信息/ }).click()
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  await page.getByRole('button', { name: /确认整理/ }).click()
  await expect(page.getByRole('heading', { name: '整理完成', exact: true })).toBeVisible()
}

const notices = (page: Page) => page.evaluate(() => (window as any).notices as { title: string; body: string; sent: boolean }[])

test('scrape and organize send a system notification while the window is in the background', async ({ page }) => {
  await setup(page, false)
  await organize(page)
  await expect.poll(async () => (await notices(page)).length).toBe(2)
  const [scrape, organized] = await notices(page)
  expect(scrape).toMatchObject({ title: '刮削完成', sent: true })
  expect(scrape.body).toMatch(/^找到 \d+ 部/)
  expect(organized).toMatchObject({ title: '整理完成', sent: true })
  expect(organized.body).toMatch(/^已整理 \d+ 部$/)
})

test('no system notification while the window is focused or the setting is off', async ({ page }) => {
  await setup(page, true)
  await organize(page)
  await expect.poll(async () => (await notices(page)).length).toBe(2)
  expect((await notices(page)).every((n) => !n.sent)).toBe(true)
})

test('the organize settings can turn finish notifications off', async ({ page }) => {
  await setup(page, false, false)
  await organize(page)
  await expect.poll(async () => (await notices(page)).length).toBe(2)
  expect((await notices(page)).every((n) => !n.sent)).toBe(true)
  await page.getByRole('navigation', { name: '主导航' }).getByRole('button', { name: '设置', exact: true }).click()
  await page.getByRole('tab', { name: /整理/ }).click()
  await expect(page.getByRole('checkbox', { name: '任务完成时发系统通知' })).not.toBeChecked()
})
