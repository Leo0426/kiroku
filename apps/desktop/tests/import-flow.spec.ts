import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

test.beforeEach(async ({ page }) => seedLibrary(page))

test('plan shows the occupied metadata paths and disables the conflicting movie', async ({ page }) => {
  await overrideApi(page, `
    const getSettings = api.getSettings;
    api.getSettings = async () => ({ ...(await getSettings()), source_dirs: ['/mock/source'] });
    const scrapeStart = api.scrapeStart;
    api.scrapeStart = async (ids) => scrapeStart(ids.filter(id => id === 11));
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await page.getByRole('button', { name: /查找影片信息/ }).click()
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  await expect(page.getByRole('checkbox', { name: '整理 ABQ-100', exact: true })).toBeDisabled()
  await expect(page.getByText(/已占用：.*\.nfo/)).toBeVisible()
  await expect(page.getByRole('button', { name: /确认整理/ })).toBeDisabled()
})

test('pending import uses the source folder just scanned in the import screen', async ({ page }) => {
  await overrideApi(page, `api.pickDirectory = async () => '/mock/source-new';`)
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '添加文件夹', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await expect(page.getByRole('button', { name: /查找影片信息/ })).toBeEnabled()
  await page.getByRole('button', { name: '媒体库', exact: true }).click()
  await page.getByRole('tab', { name: /待整理/ }).click()
  await page.getByRole('button', { name: '开始整理', exact: true }).click()
  await page.getByText(/查看已识别文件/).click()
  await expect(page.getByRole('region', { name: '扫描结果' })).toContainText('/mock/source-new')
  await expect(page.getByRole('button', { name: /查找影片信息/ })).toBeEnabled()
})

test('mock backend invalidates plans after a library change', async ({ page }) => {
  await overrideApi(page, `
    const getSettings = api.getSettings;
    api.getSettings = async () => ({ ...(await getSettings()), source_dirs: ['/mock/source'] });
    const scrapeStart = api.scrapeStart;
    api.scrapeStart = async (ids) => scrapeStart(ids.filter((id) => id === 3 || id === 4));
    const applyStart = api.applyStart;
    api.applyStart = async (ids) => {
      await api.setLibraryRoot('/mock/library-new');
      return applyStart(ids);
    };
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await page.getByRole('button', { name: /查找影片信息/ }).click()
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  await page.getByRole('button', { name: /确认整理/ }).click()
  await expect(page.getByRole('alert')).toContainText('重新生成整理计划')
  await expect(page.getByRole('heading', { name: '整理完成', exact: true })).toHaveCount(0)
})

test('a partial rollback keeps the retry action available', async ({ page }) => {
  await overrideApi(page, `
    const getSettings = api.getSettings;
    api.getSettings = async () => ({ ...(await getSettings()), source_dirs: ['/mock/source'] });
    const scrapeStart = api.scrapeStart;
    api.scrapeStart = async (ids) => scrapeStart(ids.filter((id) => id === 3));
    const rollback = api.rollback;
    let attempted = false;
    api.rollback = async (id) => {
      if (!attempted) { attempted = true; return { failures: ['destination unavailable'], status: 'partial' }; }
      return rollback(id);
    };
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await page.getByRole('button', { name: /查找影片信息/ }).click()
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  await page.getByRole('button', { name: /确认整理/ }).click()
  await page.getByRole('button', { name: '撤销本批', exact: true }).click()
  await page.getByRole('button', { name: '确认撤销', exact: true }).click()
  await expect(page.getByText('本批已撤销', { exact: true })).toHaveCount(0)
  await page.getByRole('button', { name: '撤销本批', exact: true }).click()
  await page.getByRole('button', { name: '确认撤销', exact: true }).click()
  await expect(page.getByText('本批已撤销', { exact: true })).toBeVisible()
})

test('retry requests only unsuccessful codes and preserves successful previews', async ({ page }) => {
  await overrideApi(page, `
    const getSettings = api.getSettings;
    api.getSettings = async () => ({ ...(await getSettings()), source_dirs: ['/mock/source'] });
    const scrapeStart = api.scrapeStart;
    window.scrapeCalls = [];
    api.scrapeStart = async (ids) => { window.scrapeCalls.push(ids); return scrapeStart(ids); };
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await page.getByRole('button', { name: /查找影片信息/ }).click()
  const previews = page.getByRole('region', { name: '已找到的影片' })
  await expect(page.getByRole('button', { name: /重试未完成/ })).toBeEnabled({ timeout: 30000 })
  await page.getByText(/查看已找到的影片/).click()
  await expect(previews.locator('.rc').first()).toBeVisible()
  // Cover loading can remove its placeholder during the retry. Compare the
  // movie information, which is the state this regression protects.
  const before = await previews.locator('.body').allTextContents()
  await page.getByRole('button', { name: /重试未完成/ }).click()
  await expect(page.getByRole('button', { name: /重试未完成/ })).toBeEnabled({ timeout: 30000 })
  expect(await previews.locator('.body').allTextContents()).toEqual(before)
  const calls = await page.evaluate(() => (window as unknown as { scrapeCalls: number[][] }).scrapeCalls)
  expect(calls).toHaveLength(2)
  expect(calls[1].length).toBeLessThan(calls[0].length)
  expect(calls[1]).not.toContain(3)
  expect(calls[1]).not.toContain(4)
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  await expect(page.getByRole('button', { name: /确认整理/ })).toBeEnabled()
  const pick = page.getByRole('checkbox', { name: '整理 MOVX-679', exact: true })
  await pick.uncheck()
  await page.getByRole('button', { name: '上一步', exact: true }).click()
  await page.getByRole('button', { name: /重试未完成/ }).click()
  await page.getByRole('button', { name: '停止', exact: true }).click()
  await expect(page.getByRole('heading', { name: '已停止', exact: true })).toBeVisible()
  await page.getByText(/查看已找到的影片/).click()
  await expect(previews.locator('.rc').first()).toBeVisible()
  await expect(previews.locator('.body')).toHaveText(before)
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  await expect(pick).not.toBeChecked()
  await expect(page.getByRole('button', { name: /确认整理/ })).toBeEnabled()
})

test('a new version of an existing movie is planned unticked until the user opts in', async ({ page }) => {
  await overrideApi(page, `
    const getSettings = api.getSettings;
    api.getSettings = async () => ({ ...(await getSettings()), source_dirs: ['/mock/source'] });
    const scrapeStart = api.scrapeStart;
    api.scrapeStart = async (ids) => scrapeStart(ids.filter(id => id === 11));
    const planBuild = api.planBuild;
    api.planBuild = async (...args) => (await planBuild(...args)).map(p => p.code !== 'ABQ-100' ? p : { ...p, to: p.to.replace(/[^/]+$/, 'ABQ-100 - 中字.mp4'), extras: [], conflict: null, conflicts: [], selected: false, newVersion: true, notices: ['库中已有该分片的其他版本，将作为新版本并存：ABQ-100 - 中字.mp4；确认后请手动勾选'] });
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await page.getByRole('button', { name: /查找影片信息/ }).click()
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  const box = page.getByRole('checkbox', { name: '整理 ABQ-100', exact: true })
  await expect(box).toBeEnabled()
  await expect(box).not.toBeChecked()
  await expect(page.getByText('新版本 · 需手动勾选')).toBeVisible()
  await expect(page.getByText(/ABQ-100 - 中字\.mp4/)).toBeVisible()
  await expect(page.getByRole('button', { name: /确认整理/ })).toBeDisabled()
  await box.check()
  await expect(page.getByRole('button', { name: /确认整理/ })).toBeEnabled()
})
