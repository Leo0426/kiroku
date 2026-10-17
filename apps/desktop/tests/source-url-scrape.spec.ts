import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

test.beforeEach(async ({ page }) => seedLibrary(page))

test('a movie not found by search can be scraped from a given detail page', async ({ page }) => {
  test.setTimeout(120_000)
  await overrideApi(page, `
    const getSettings = api.getSettings;
    api.getSettings = async () => ({ ...(await getSettings()), source_dirs: ['/mock/source'] });
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await page.getByRole('button', { name: /查找影片信息/ }).click()
  const issues = page.getByRole('region', { name: '需要处理的影片' })
  // mock 逐部查询约 1 秒,等整批结束
  await expect(page.getByRole('button', { name: /重试未完成/ })).toBeVisible({ timeout: 60_000 })
  await expect(issues).toContainText('PRVW-455')

  await issues.getByRole('button', { name: '为 PRVW-455 指定详情页' }).click()
  await issues.getByLabel('PRVW-455 详情页地址').fill('https://evil.example/v/x')
  await issues.getByRole('button', { name: '用详情页刮削', exact: true }).click()
  await expect(issues).toContainText('不是已配置的数据源域名')

  await issues.getByRole('button', { name: '为 PRVW-455 指定详情页' }).click()
  await issues.getByLabel('PRVW-455 详情页地址').fill('https://javdb.com/v/PRVW-455')
  await issues.getByRole('button', { name: '用详情页刮削', exact: true }).click()
  await expect(issues).not.toContainText('PRVW-455', { timeout: 15_000 })
  await page.getByText('查看已找到的影片').click()
  await expect(page.getByRole('region', { name: '已找到的影片' })).toContainText('PRVW-455')
})

test('detail page can refresh selected fields from a given detail page', async ({ page }) => {
  await page.goto('/')
  await page.locator('button.pcard').first().click()
  const panel = page.getByLabel('影片详情', { exact: true })
  await panel.getByRole('button', { name: '更新与来源', exact: true }).click()
  await panel.getByLabel('详情页地址（可选）').fill('https://evil.example/x')
  await panel.getByRole('button', { name: '预览更新', exact: true }).click()
  await expect(panel).toContainText('不是已配置的数据源域名')
  await panel.getByLabel('详情页地址（可选）').fill('https://www.jav321.com/video/x')
  await panel.getByRole('button', { name: '预览更新', exact: true }).click()
  await expect(panel.getByLabel('更新差异预览', { exact: true })).toBeVisible()
})
