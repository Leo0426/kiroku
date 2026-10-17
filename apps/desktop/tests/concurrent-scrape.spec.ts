import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

test('scrape progress names every site being queried at once', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    const getSettings = api.getSettings;
    api.getSettings = async () => ({ ...(await getSettings()), source_dirs: ['/mock/source'] });
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await page.getByRole('button', { name: /查找影片信息/ }).click()
  await expect(page.getByRole('region', { name: '正在查询的影片' })).toContainText(/正在查询 JavDB、JavBus/)
})
