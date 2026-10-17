import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

test.beforeEach(async ({ page }) => seedLibrary(page))

// 130 条合成记录,按 id 从新到旧;后端门面按游标分页。
const PAGED = `
  const rows = Array.from({ length: 130 }, (_, i) => ({
    id: String(2000 - i), ts: 1780000000000 - i * 60000, status: 'applied', itemCount: 1, opCount: 1,
    codes: ['HIS-' + String(i).padStart(3, '0')], trashed: 0, pendingOperations: 0, error: null,
  }));
  window.historyCalls = [];
  api.historyList = async (_id, page = {}) => {
    window.historyCalls.push(page);
    const start = page.before ? rows.findIndex(r => r.id < page.before) : 0;
    const from = start < 0 ? rows.length : start;
    const items = rows.slice(from, from + (page.limit ?? 50));
    const end = from + items.length;
    return { items, total: rows.length, next: end < rows.length && items.length ? items[items.length - 1].id : null };
  };
  api.rollback = async () => ({ indexError: null, failures: [], status: 'rolledBack' });
`

const cards = (page: import('@playwright/test').Page) => page.getByRole('list', { name: '操作记录' }).getByRole('listitem')

test('history shows the newest page first and loads older records on request', async ({ page }) => {
  await overrideApi(page, PAGED)
  await page.goto('/')
  await page.getByRole('button', { name: '历史', exact: true }).click()
  await expect(cards(page)).toHaveCount(50)
  await expect(page.getByText('HIS-000', { exact: true })).toBeVisible()
  await expect(page.getByText('HIS-050', { exact: true })).toHaveCount(0)
  await page.getByRole('button', { name: /加载更早的记录.*还有 80 条/ }).click()
  await expect(cards(page)).toHaveCount(100)
  await page.getByRole('button', { name: /加载更早的记录.*还有 30 条/ }).click()
  await expect(cards(page)).toHaveCount(130)
  await expect(page.getByText('HIS-129', { exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: /加载更早的记录/ })).toHaveCount(0)
})

test('undo on an older page refreshes without collapsing the loaded records', async ({ page }) => {
  await overrideApi(page, PAGED)
  await page.goto('/')
  await page.getByRole('button', { name: '历史', exact: true }).click()
  await page.getByRole('button', { name: /加载更早的记录/ }).click()
  await expect(cards(page)).toHaveCount(100)
  const row = cards(page).filter({ hasText: 'HIS-075' })
  await row.getByRole('button', { name: '撤销' }).click()
  await row.getByRole('button', { name: '确认撤销' }).click()
  await expect(page.getByText(/已撤销，文件已按操作日志恢复/)).toBeVisible()
  await expect(cards(page)).toHaveCount(100)
  const calls = await page.evaluate(() => (window as any).historyCalls)
  expect(calls.at(-1)).toEqual({ limit: 100 })
})
