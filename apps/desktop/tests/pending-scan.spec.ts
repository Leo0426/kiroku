import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

test('library counts use summaries, reuse cache across navigation and refresh previews explicitly', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    const settings = await api.getSettings(); settings.source_dirs = ['/mock/source']; await api.saveSettings(settings);
    window.countReads = []; window.previewReads = [];
    const count = api.pendingSummary; const preview = api.pendingList;
    api.pendingSummary = async (...args) => { const value = await count(...args); window.countReads.push(value); return value };
    api.pendingList = async (...args) => { const value = await preview(...args); window.previewReads.push(value); return value };
  `)
  await page.goto('/')
  await expect.poll(() => page.evaluate(() => ((window as any).countReads?.length ?? 0))).toBe(1)
  expect(await page.evaluate(() => (window as any).previewReads.length)).toBe(0)
  await page.getByRole('button', { name: '历史', exact: true }).click()
  await page.getByRole('button', { name: '媒体库', exact: true }).click()
  await expect.poll(() => page.evaluate(() => ((window as any).countReads?.length ?? 0))).toBe(2)
  expect(await page.evaluate(() => (window as any).countReads[1].cached)).toBe(true)
  await page.getByRole('tab', { name: /待整理/ }).click()
  await expect(page.locator('section[aria-label="待整理的视频"]')).toBeVisible()
  await page.getByRole('button', { name: '刷新待整理', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (window as any).previewReads.length)).toBe(2)
  expect(await page.evaluate(() => (window as any).previewReads[1].cached)).toBe(false)
})

test('partial and cancelled previews cannot appear as complete empty folders', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    const settings = await api.getSettings(); settings.source_dirs = ['/mock/missing']; await api.saveSettings(settings);
    window.cancelResult = false;
    api.pendingList = async () => ({ dirs: ['/mock/missing'], items: [], total: 0, codes: 0, unrecognized: 0,
      skippedTotal: 0, visited: 0, errors: window.cancelResult ? [] : [{path:'/mock/missing',message:'permission denied'}],
      errorsTotal: window.cancelResult ? 0 : 1, cancelled: window.cancelResult, complete:false, truncated:false, checkedAt:Date.now(), cached:false });
  `)
  await page.goto('/')
  await page.getByRole('tab', { name: /待整理/ }).click()
  await expect(page.locator('.pending').getByRole('alert')).toContainText('/mock/missing')
  await expect(page.locator('.pending').getByRole('alert')).toContainText('permission denied')
  await expect(page.getByText('没有待整理的视频', { exact: true })).toHaveCount(0)
  await page.evaluate(() => (window as any).cancelResult = true)
  await page.getByRole('button', { name: '刷新待整理', exact: true }).click()
  await expect(page.locator('.pending').getByRole('alert')).toContainText('检查已停止')
  await expect(page.getByText('没有待整理的视频', { exact: true })).toHaveCount(0)
})

test('leaving a pending page cancels only its request and discards late data', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    window.cancels = []; window.requestId = null;
    const preview = api.pendingList;
    api.pendingList = async requestId => {
      window.requestId = requestId;
      return await new Promise(resolve => window.releasePending = async () => resolve(await preview(requestId)));
    };
    api.pendingCancel = async requestId => { window.cancels.push(requestId); return true };
  `)
  await page.goto('/')
  await page.getByRole('tab', { name: /待整理/ }).click()
  await expect.poll(() => page.evaluate(() => typeof (window as any).requestId)).toBe('string')
  const requestId = await page.evaluate(() => (window as any).requestId)
  await page.getByRole('button', { name: '历史', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (window as any).cancels)).toContain(requestId)
  await page.evaluate(() => (window as any).releasePending())
  await expect(page.locator('.pending')).toHaveCount(0)
})

test('scan errors remain visible and do not claim that no video exists', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    const settings = await api.getSettings(); settings.source_dirs = ['/mock/missing']; await api.saveSettings(settings);
    const { emitMock } = await import('/src/lib/events.ts');
    api.scanStart = async () => {
      emitMock('scan:done', {jobId:900,total:0,codes:0,unrecognized:0,skipped:[],skippedTotal:0,cancelled:false,
        complete:false,errorsTotal:1,errors:[{path:'/mock/missing',message:'source disconnected'}]}); return 900;
    };
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('source disconnected')
  await expect(page.getByText('没有找到视频文件。请选择其他文件夹，或检查扫描过滤规则。', { exact: true })).toHaveCount(0)
})

test('cancelling import while background reads drain does not start a new scan', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    const settings = await api.getSettings(); settings.source_dirs = ['/mock/source']; await api.saveSettings(settings);
    window.fullScanCalls = 0;
    const count = api.pendingSummary;
    api.pendingSummary = async (...args) => { await new Promise(resolve => window.releaseCount = resolve); return count(...args) };
    api.pendingCancel = async () => true;
    api.scanStart = async () => { window.fullScanCalls++; return 950 };
  `)
  await page.goto('/')
  await expect.poll(() => page.evaluate(() => typeof (window as any).releaseCount)).toBe('function')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await page.getByRole('button', { name: '停止', exact: true }).click()
  await page.evaluate(() => (window as any).releaseCount())
  await expect(page.getByText('扫描已停止，仅显示已读到的文件。', { exact: true })).toBeVisible()
  expect(await page.evaluate(() => (window as any).fullScanCalls)).toBe(0)
})

test('cancellation while scanStart is pending targets the returned job id', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    const settings = await api.getSettings(); settings.source_dirs = ['/mock/source']; await api.saveSettings(settings);
    const { emitMock } = await import('/src/lib/events.ts');
    window.cancelledIds = [];
    api.scanStart = async () => await new Promise(resolve => window.releaseScan = () => resolve(960));
    api.jobCancel = async jobId => {
      window.cancelledIds.push(jobId);
      emitMock('scan:done', {jobId:960,total:0,codes:0,unrecognized:0,skipped:[],skippedTotal:0,
        cancelled:true,complete:false,errors:[],errorsTotal:0}); return true;
    };
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await expect.poll(() => page.evaluate(() => typeof (window as any).releaseScan)).toBe('function')
  await page.getByRole('button', { name: '停止', exact: true }).click()
  expect(await page.evaluate(() => (window as any).cancelledIds)).toEqual([])
  await page.evaluate(() => (window as any).releaseScan())
  await expect.poll(() => page.evaluate(() => (window as any).cancelledIds)).toEqual([960])
  await expect(page.getByText('扫描已停止，仅显示已读到的文件。', { exact: true })).toBeVisible()
})
