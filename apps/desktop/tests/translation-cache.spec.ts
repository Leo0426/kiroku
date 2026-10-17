import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

test('the AI settings can clear the translation cache', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `window.cleared = 0; api.clearTranslationCache = async () => { window.cleared++ };`)
  await page.goto('/')
  await page.getByRole('navigation', { name: '主导航' }).getByRole('button', { name: '设置', exact: true }).click()
  await page.getByRole('tab', { name: /AI 汉化/ }).click()
  await page.getByRole('button', { name: '清空译文缓存', exact: true }).click()
  await expect(page.getByText('译文缓存已清空')).toBeVisible()
  expect(await page.evaluate(() => (window as any).cleared)).toBe(1)
})
