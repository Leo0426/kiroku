import { expect, test } from '@playwright/test'
import { seedLibrary } from './helpers'

test('the built-in tag preset merges into name rules and maps Japanese tags after saving', async ({ page }) => {
  await seedLibrary(page)
  await page.goto('/')
  await page.getByRole('navigation', { name: '主导航' }).getByRole('button', { name: '设置', exact: true }).click()
  await page.getByRole('tab', { name: '名称规则', exact: true }).click()
  await page.getByRole('button', { name: '导入内置标签译名', exact: true }).click()
  await expect(page.getByRole('status')).toContainText(/已加入 \d+ 条标签规则、\d+ 个别名/)
  await expect(page.getByLabel('名称类别')).toHaveValue('tags')
  await expect(page.locator('.rule').filter({ hasText: '剧情' })).toContainText('ドラマ')
  // 再导入一次不会重复
  await page.getByRole('button', { name: '导入内置标签译名', exact: true }).click()
  await expect(page.getByRole('status')).toContainText('内置标签译名已全部在规则里')

  await page.getByRole('button', { name: /保存更改/ }).click()
  await page.getByRole('button', { name: '读取库名称', exact: true }).click()
  const rows = page.locator('[data-catalog-movie]')
  for (let i = 0; i < 6; i++) await rows.nth(i).getByRole('checkbox').check()
  await page.getByRole('button', { name: '预览所选影片', exact: true }).click()
  const diffs = page.locator('[data-name-diff]')
  await expect(diffs.first()).toBeVisible()
  await expect(diffs.filter({ hasText: /tags：.*→/ }).first()).toContainText(/剧情|单体作品|高画质|独家配信|恋爱|纪录片|眼镜|短发/)
})
