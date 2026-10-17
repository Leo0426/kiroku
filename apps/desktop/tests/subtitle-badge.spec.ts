import { expect, test } from '@playwright/test'
import { seedLibrary } from './helpers'

test('external Chinese subtitles count as subtitled by default and the rule can be turned off', async ({ page }) => {
  await seedLibrary(page)
  await page.goto('/')
  await page.getByRole('navigation', { name: '主导航' }).getByRole('button', { name: '设置', exact: true }).click()
  await page.getByRole('tab', { name: /封面角标/ }).click()
  await page.getByRole('checkbox', { name: '给封面加角标' }).check()
  const rule = page.getByRole('checkbox', { name: /外挂中文字幕也算中字/ })
  await expect(rule).toBeChecked()
  await rule.uncheck()
  await page.getByRole('button', { name: /保存更改/ }).click()
  await expect(page.getByRole('button', { name: '已保存', exact: true })).toBeVisible()
  const saved = await page.evaluate(() => JSON.parse(localStorage.getItem('kiroku-mock-settings-v2')!))
  expect(saved.badges.external_subtitle).toBe(false)
})
