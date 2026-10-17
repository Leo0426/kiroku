import { expect, test } from '@playwright/test'
import { seedLibrary } from './helpers'

test.beforeEach(async ({ page }) => seedLibrary(page))

test('right-clicking a poster offers movie actions and both delete modes remove it after confirmation', async ({ page }) => {
  await page.goto('/')
  const first = page.locator('button.pcard').first()
  const code = (await first.locator('.sub .mono').textContent())!.trim()
  const card = page.locator('button.pcard').filter({ hasText: code })

  await first.click({ button: 'right' })
  const menu = page.getByRole('menu', { name: `${code} 操作` })
  for (const name of ['查看详情', '播放', '在文件夹中显示', '收藏', '只删除资料…', '删除影片和文件…']) await expect(menu.getByRole('menuitem', { name, exact: true })).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(menu).toHaveCount(0)

  await first.click({ button: 'right' })
  await menu.getByRole('menuitem', { name: '收藏', exact: true }).click()
  await expect(card.getByLabel('已收藏')).toBeVisible()

  await card.click({ button: 'right' })
  await menu.getByRole('menuitem', { name: '删除影片和文件…' }).click()
  const dialog = page.getByRole('alertdialog')
  await expect(dialog).toContainText('整个影片文件夹')
  await dialog.getByRole('button', { name: '取消' }).click()
  await expect(card).toHaveCount(1)

  await card.click({ button: 'right' })
  await menu.getByRole('menuitem', { name: '删除影片和文件…' }).click()
  await dialog.getByRole('button', { name: '删除影片和文件' }).click()
  await expect(page.getByText(`已将 ${code} 移到废纸篓`)).toBeVisible()
  await expect(card).toHaveCount(0)

  const next = page.locator('button.pcard').first()
  const other = (await next.locator('.sub .mono').textContent())!.trim()
  await next.click({ button: 'right' })
  await page.getByRole('menu', { name: `${other} 操作` }).getByRole('menuitem', { name: '只删除资料…' }).click()
  await expect(dialog).toContainText('视频和字幕文件留在原文件夹')
  await dialog.getByRole('button', { name: '删除资料' }).click()
  await expect(page.getByText(`已将 ${other} 的资料移到废纸篓，视频保留`)).toBeVisible()
  await expect(page.locator('button.pcard').filter({ hasText: other })).toHaveCount(0)
})
