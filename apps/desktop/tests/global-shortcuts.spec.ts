import { expect, test } from '@playwright/test'
import { seedLibrary } from './helpers'

test.beforeEach(async ({ page }) => {
  await seedLibrary(page)
  await page.goto('/')
})

test('global setting search locates the precise control and preserves edits within settings', async ({ page }) => {
  const quick = page.getByRole('button', { name: '快捷操作', exact: true })
  const search = page.getByRole('combobox', { name: '搜索操作' })
  await quick.click()
  await search.fill('头像')
  await search.press('Enter')
  const hosting = page.getByRole('checkbox', { name: '启用演员头像托管' })
  await expect(hosting).toBeFocused()
  await expect(hosting).not.toBeChecked()

  await page.keyboard.press('Control+k')
  await search.fill('目录模板')
  await search.press('Enter')
  const template = page.getByLabel('目录模板', { exact: true })
  await expect(template).toBeFocused()
  await template.fill('{code}/Draft')
  await page.keyboard.press('Control+k')
  await search.fill('缓存')
  await page.getByRole('option', { name: '抓取缓存' }).click()
  await expect(page.getByRole('region', { name: '抓取缓存' })).toBeFocused()
  await page.getByRole('tab', { name: '命名与 NFO' }).click()
  await expect(template).toHaveValue('{code}/Draft')
})

test('the bottom bar opens shortcuts from history without a keyboard', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 680 })
  const nav = page.getByRole('navigation', { name: '主导航' })
  await nav.getByRole('button', { name: '历史', exact: true }).click()
  const quick = nav.getByRole('button', { name: '快捷操作' })
  await quick.click()
  const search = page.getByRole('combobox', { name: '搜索操作' })
  await search.fill('字体')
  await page.getByRole('option', { name: '文字大小' }).click()
  await expect(page.getByLabel('文字大小', { exact: true })).toBeFocused()
  await expect(quick).toBeInViewport()
  expect(await quick.evaluate(el => el.clientWidth)).toBeGreaterThanOrEqual(44)
})
