import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

test.beforeEach(async ({ page }) => seedLibrary(page))

test('scan prioritizes unknown files while recognized files remain editable on demand', async ({ page }) => {
  await overrideApi(page, `const get = api.getSettings; api.getSettings = async () => ({ ...(await get()), source_dirs: ['/mock/source'] });`)
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await expect(page.getByText(/mp4 \/ mkv/)).not.toBeVisible()
  await page.getByRole('button', { name: '待整理的文件夹说明', exact: true }).hover()
  await expect(page.getByText(/mp4 \/ mkv/)).toBeVisible()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await expect(page.getByRole('button', { name: /查找影片信息/ })).toBeEnabled()
  await expect(page.getByRole('region', { name: '需要确认的文件' })).toBeVisible()
  await expect(page.getByRole('button', { name: /修改番号/ })).toHaveCount(0)
  await page.getByText(/查看已识别文件/).click()
  await expect(page.getByRole('button', { name: /修改番号/ }).first()).toBeVisible()
  const unknown = page.getByRole('region', { name: '需要确认的文件' })
  const before = await unknown.getByRole('textbox').count()
  const row = unknown.locator('.scanrow').first()
  await row.getByRole('textbox').fill('ABC-999')
  await row.getByRole('button', { name: '确定', exact: true }).click()
  await expect(unknown.getByRole('textbox')).toHaveCount(before - 1)
  await expect(page.getByRole('button', { name: '修改番号 ABC-999', exact: true })).toBeVisible()
})

test('lookup shows failures first and keeps full results behind disclosures', async ({ page }) => {
  await overrideApi(page, `
    const get = api.getSettings; api.getSettings = async () => ({ ...(await get()), source_dirs: ['/mock/source'] });
    const scrape = api.scrapeStart; api.scrapeStart = ids => scrape(ids.filter(id => id === 2 || id === 5));
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await page.getByRole('button', { name: /查找影片信息/ }).click()
  await expect(page.getByRole('button', { name: '预览整理', exact: true })).toBeEnabled()
  await expect(page.getByRole('region', { name: '需要处理的影片' })).toBeVisible()
  await expect(page.getByRole('region', { name: '需要处理的影片' })).toContainText('FC2-')
  await expect(page.getByRole('region', { name: '需要处理的影片' })).not.toContainText('MOVX-679')
  await expect(page.getByRole('region', { name: '已找到的影片' }).locator('.rc')).toHaveCount(0)
  await page.getByText(/查看已找到的影片/).click()
  await expect(page.getByRole('region', { name: '已找到的影片' }).locator('.rc')).toHaveCount(1)
  await page.getByText('查看全部查询状态', { exact: true }).click()
  await expect(page.getByRole('region', { name: '全部查询状态' })).toContainText('MOVX-679')
})

for (const [mode, label] of [['move', '移动'], ['copy', '复制'], ['hardlink', '硬链接'], ['symlink', '软链接']] as const) {
  test(`${mode} import explains the actual file operation before confirmation`, async ({ page }) => {
    await overrideApi(page, `
      const s = await api.getSettings(); s.source_dirs = ['/mock/source']; s.organize_mode = '${mode}'; s.cleanup_source = true; await api.saveSettings(s);
      const scrape = api.scrapeStart; api.scrapeStart = ids => scrape(ids.filter(id => id === 3));
      api.applyStart = async () => new Promise(r => window.releaseApply = r);
    `)
    await page.goto('/')
    await page.getByRole('button', { name: '导入整理', exact: true }).click()
    const destination = page.getByRole('region', { name: '整理目标' })
    await expect(destination).toContainText(label)
    if (mode !== 'move') await expect(destination).toContainText('保留')
    await page.getByRole('button', { name: '开始扫描', exact: true }).click()
    await page.getByRole('button', { name: /查找影片信息/ }).click()
    await page.getByRole('button', { name: '预览整理', exact: true }).click()
    const hint = page.getByRole('note', { name: '文件处理方式' })
    await expect(hint).toContainText(label)
    if (mode === 'move') await expect(hint).toContainText('废纸篓')
    else { await expect(hint).toContainText('保留'); await expect(hint).not.toContainText('废纸篓') }
    await page.getByRole('button', { name: /确认整理/ }).click()
    await expect(page.getByRole('status', { name: '当前整理方式' })).toContainText(label)
    await page.evaluate(async () => {
      const { app } = await import('/src/lib/state.svelte.ts' as string)
      app.settings.organize_mode = app.settings.organize_mode === 'move' ? 'copy' : 'move'
    })
    await expect(page.getByRole('status', { name: '当前整理方式' })).toContainText(label)
  })
}
