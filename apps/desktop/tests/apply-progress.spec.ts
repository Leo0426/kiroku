import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

test('download progress accepts early events, survives navigation and rejects other jobs', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    const settings = await api.getSettings(); settings.source_dirs = ['/mock/source']; await api.saveSettings(settings);
    const scrape = api.scrapeStart; api.scrapeStart = ids => scrape(ids.filter(id => id === 3));
    const { emitMock } = await import('/src/lib/events.ts');
    window.progress = payload => emitMock('apply:progress', { jobId: 700, phase: 'images', done: 0, total: 12, failed: 0, code: 'QRS-912', ...payload });
    api.applyStart = async () => {
      window.progress({});
      await new Promise(resolve => window.releaseApply = resolve);
      return 700;
    };
    api.jobCancel = async () => {
      emitMock('apply:done', { jobId: 700, applied: 0, failed: [], cancelled: true, journalId: 'mock-cancelled', cleaned: [], indexError: null });
      window.releaseApply();
    };
  `)
  await page.goto('/')
  await page.getByRole('button', { name: /^导入整理/ }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await page.getByRole('button', { name: /查找影片信息/ }).click()
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  await page.getByRole('button', { name: /确认整理/ }).click()
  await expect(page.getByRole('heading', { name: '正在下载图片', exact: true })).toBeVisible()
  await expect(page.getByRole('progressbar', { name: '图片下载进度' })).toHaveAttribute('aria-valuenow', '0')
  await page.evaluate(() => (window as any).progress({ done: 6, failed: 1 }))
  await expect(page.getByRole('progressbar', { name: '图片下载进度' })).toHaveAttribute('aria-valuenow', '50')
  await expect(page.getByRole('status', { name: '图片下载状态' })).toContainText('QRS-912')
  await expect(page.getByRole('status', { name: '图片下载状态' })).toContainText('1 张未下载成功')
  await page.evaluate(() => (window as any).progress({ jobId: 701, done: 12 }))
  await expect(page.getByRole('progressbar', { name: '图片下载进度' })).toHaveAttribute('aria-valuenow', '50')
  await page.getByRole('button', { name: '媒体库', exact: true }).click()
  await page.evaluate(() => (window as any).progress({ done: 9 }))
  await page.getByRole('button', { name: /^导入整理/ }).click()
  await expect(page.getByRole('progressbar', { name: '图片下载进度' })).toHaveAttribute('aria-valuenow', '75')
  await page.evaluate(() => (window as any).progress({ phase: 'organize', done: 0, total: 1, code: null }))
  await expect(page.getByRole('heading', { name: '正在整理', exact: true })).toBeVisible()
  await expect(page.getByRole('progressbar', { name: '图片下载进度' })).toHaveCount(0)
  await page.evaluate(() => (window as any).progress({ phase: 'index', done: 0, total: 1, code: null }))
  await expect(page.getByRole('heading', { name: '正在更新媒体库', exact: true })).toBeVisible()
  await page.getByRole('button', { name: '停止', exact: true }).click()
  await expect(page.getByRole('heading', { name: '已停止', exact: true })).toBeVisible()
  await page.evaluate(() => (window as any).progress({ done: 12 }))
  await expect(page.getByRole('heading', { name: '已停止', exact: true })).toBeVisible()
})

test('mock keeps still downloads and finishes the same progress phases as desktop', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    const settings = await api.getSettings(); settings.source_dirs = ['/mock/source']; settings.download_extrafanart = true; await api.saveSettings(settings);
    const scrape = api.scrapeStart; api.scrapeStart = ids => scrape(ids.filter(id => id === 3));
    const { listen } = await import('/src/lib/events.ts');
    window.downloadEvents = [];
    await listen('apply:progress', event => window.downloadEvents.push(event));
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await page.getByRole('button', { name: /查找影片信息/ }).click()
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  await page.getByRole('button', { name: /确认整理/ }).click()
  await expect(page.getByRole('heading', { name: '整理完成', exact: true })).toBeVisible()
  const events = await page.evaluate(() => (window as any).downloadEvents as { jobId: number; phase: string; done: number; total: number }[])
  const images = events.filter(e => e.phase === 'images')
  expect(images[0].done).toBe(0)
  expect(images.at(-1)?.done).toBe(5) // poster, fanart and three stills
  expect(images.every(e => e.total === 5 && e.done <= e.total)).toBe(true)
  expect(events.slice(-2).map(e => e.phase)).toEqual(['organize', 'index'])
  expect(new Set(events.map(e => e.jobId)).size).toBe(1)
})
