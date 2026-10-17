import { expect, test } from '@playwright/test'
import { seedLibrary } from './helpers'

test.beforeEach(async ({ page }) => seedLibrary(page))

test('a movie can be favorited from its detail panel, filtered, and the change undone from history', async ({ page }) => {
  await page.goto('/')
  const first = page.locator('button.pcard').first()
  const code = (await first.locator('.sub .mono').textContent())!.trim()
  await first.click()
  const panel = page.getByLabel('影片详情', { exact: true })
  await panel.getByRole('button', { name: '收藏', exact: true }).click()
  await expect(panel.getByRole('button', { name: '取消收藏', exact: true })).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(page.locator('button.pcard').filter({ hasText: code }).getByLabel('已收藏')).toBeVisible()

  await page.getByRole('button', { name: '筛选', exact: true }).click()
  await page.getByRole('checkbox', { name: /只看收藏/ }).check()
  await expect(page.locator('button.pcard')).toHaveCount(1)
  await expect(page.locator('button.pcard').first()).toContainText(code)

  await page.getByRole('button', { name: '历史', exact: true }).click()
  const row = page.getByRole('list', { name: '操作记录' }).getByRole('listitem').filter({ hasText: code }).first()
  await row.getByRole('button', { name: '撤销' }).click()
  await row.getByRole('button', { name: '确认撤销' }).click()
  await expect(page.getByText(/已撤销/).first()).toBeVisible()
  await page.getByRole('button', { name: '媒体库', exact: true }).click()
  await expect(page.locator('button.pcard').filter({ hasText: code }).getByLabel('已收藏')).toHaveCount(0)
})
