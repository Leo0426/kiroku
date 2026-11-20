import { expect, test, type Page } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'
import type { ApplyDone } from '../src/lib/types'

async function finishBatch(page: Page, outcome: Partial<ApplyDone>, warnings: string[] = []) {
  await seedLibrary(page)
  await overrideApi(page, `
    const { emitMock } = await import('/src/lib/events.ts');
    api.applyStart = async () => {
      emitMock('apply:event', { jobId: 910, itemId: 0, code: 'SMPL-421', done: 1, total: 1, ok: true, error: null, warnings: ${JSON.stringify(warnings)} });
      emitMock('apply:done', {
        jobId: 910, applied: 1, failed: [], cancelled: false, journalId: 'batch-910', canRollback: true,
        cleaned: [], indexError: null, ...${JSON.stringify(outcome)}
      });
      return 910;
    };
    api.notifyFinish = async (title, body) => {
      window.applyNotice = { title, body }; return true;
    };
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  page.once('dialog', dialog => dialog.accept('/mock/companion-import/SMPL-421.mp4'))
  await page.getByRole('button', { name: '添加视频文件' }).click()
  await page.getByRole('button', { name: '开始扫描' }).click()
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  await page.getByRole('button', { name: /确认整理/ }).click()
}

test('a placed video with a later write failure is reported as partial in the window and notification', async ({ page }) => {
  await finishBatch(page, { failed: [['SMPL-421', 'NFO 写入失败']], journalId: null, canRollback: false })
  await expect(page.getByRole('heading', { name: '整理部分完成', exact: true })).toBeVisible()
  await expect(page.getByText('NFO 写入失败', { exact: true })).toBeVisible()
  const result = page.getByLabel('本批整理结果')
  await expect(result).toContainText('1 项已写入视频或字幕')
  await expect(result).toContainText('1 条执行错误')
  await expect(page.getByRole('button', { name: '撤销本批' })).toHaveCount(0)
  const notice = await page.evaluate(() => (window as unknown as { applyNotice: { title: string; body: string } }).applyNotice)
  expect(notice.title).toBe('整理部分完成')
  await expect(result).toContainText(notice.body)
})

test('missing artwork keeps the video result and reveals the file-specific reason', async ({ page }) => {
  await finishBatch(page, { imageFailures: 2 }, ['SMPL-421-poster.jpg 没有可用图片'])
  await expect(page.getByRole('heading', { name: '整理完成，有图片待补' })).toBeVisible()
  const result = page.getByLabel('本批整理结果')
  await expect(result).toContainText('1 项已写入视频或字幕，2 张图片准备失败')
  await result.getByText('查看文件提示', { exact: true }).click()
  await expect(result.getByText('SMPL-421-poster.jpg 没有可用图片', { exact: true })).toBeVisible()
  await expect(result).toContainText('SMPL-421.mp4')
})

test('index failure directs the user to the library index settings', async ({ page }) => {
  await finishBatch(page, { applied: 0, canRollback: true, indexError: '索引暂时被占用' })
  await expect(page.getByRole('heading', { name: '媒体库索引待更新' })).toBeVisible()
  await expect(page.getByRole('alert').filter({ hasText: '索引暂时被占用' })).toBeVisible()
  await expect(page.getByRole('button', { name: '撤销本批' })).toBeEnabled()
  await page.getByRole('button', { name: '前往索引设置' }).click()
  await expect(page.locator('#library-location')).toBeFocused()
  await expect(page.getByRole('button', { name: '重建索引', exact: true })).toBeVisible()
})

test('file notes remain accessible by keyboard when only the final artwork write had a warning', async ({ page, browserName }) => {
  await page.setViewportSize({ width: 480, height: 600 })
  const warnings = Array.from({ length: 30 }, (_, i) => `示例图片 ${i + 1} 写入失败`)
  await finishBatch(page, { imageFailures: 0 }, warnings)
  const result = page.getByLabel('本批整理结果')
  await expect(result.getByRole('heading', { name: '整理完成', exact: true })).toBeVisible()
  await expect(result).toContainText('1 项有文件提示')
  const disclosure = result.getByText('查看文件提示', { exact: true })
  await disclosure.focus()
  await page.keyboard.press('Enter')
  await page.keyboard.press(browserName === 'webkit' && process.platform === 'darwin' ? 'Alt+Tab' : 'Tab')
  const notes = result.getByRole('region', { name: '文件提示明细' })
  await expect(notes).toBeFocused()
  await expect(notes.getByText(warnings[0], { exact: true })).toBeVisible()
  await page.keyboard.press('PageDown')
  await expect.poll(() => notes.evaluate(el => el.scrollTop)).toBeGreaterThan(0)
  const lastNote = notes.getByText(warnings[29], { exact: true })
  await expect(async () => {
    await page.keyboard.press('PageDown')
    await expect(lastNote).toBeInViewport({ timeout: 400 })
  }).toPass({ timeout: 8000 })
  expect(await result.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true)
  expect(await page.evaluate(() => (window as unknown as { applyNotice: { body: string } }).applyNotice.body)).toContain('1 项有文件提示')
})
