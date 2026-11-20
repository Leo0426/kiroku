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
  await expect(page.getByRole('button', { name: '开始整理', exact: true })).toBeDisabled()
  await page.getByRole('button', { name: '返回当前导入', exact: true }).click()
  await page.getByRole('button', { name: '重新选择', exact: true }).click()
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
  await page.getByRole('button', { name: '重试撤销', exact: true }).click()
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

test('preview filters conflicts and new versions without changing checks outside the current results', async ({ page }) => {
  await overrideApi(page, `
    const settings = api.getSettings;
    api.getSettings = async () => ({ ...await settings(), source_dirs: ['/mock/companion-import'] });
    const plan = api.planBuild;
    api.planBuild = async (...args) => {
      const [sample] = await plan(...args);
      return ['NORMAL-001', 'EXTRA-002', 'EXTRA-003', 'BUSY-004'].map((code, i) => ({
        ...sample, code, itemId: i, title: '样例影片 ' + i, from: '/mock/source/' + code, to: '/mock/library/' + code,
        selected: i === 0, newVersion: i === 1 || i === 2, conflict: i === 3 ? 'targetExists' : null,
        conflicts: [], notices: [], extras: [],
      }))
    };
    window.appliedIds = null;
    api.applyStart = async ids => { window.appliedIds = ids; return 10001 };
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  const plan = page.getByRole('region', { name: '整理计划' })
  await plan.getByRole('button', { name: /有冲突/ }).click()
  await expect(plan.getByRole('checkbox')).toHaveCount(1)
  await expect(plan.getByRole('checkbox')).toBeDisabled()
  await expect(plan.getByText(/其中 1 个在筛选结果外/)).toBeVisible()
  await plan.getByRole('button', { name: /新版本/ }).click()
  await expect(plan.getByRole('checkbox')).toHaveCount(2)
  const search = plan.getByRole('searchbox', { name: '搜索整理清单' })
  await search.fill('extra-003')
  await plan.getByRole('button', { name: '全选当前结果' }).click()
  await expect(plan.getByText('新版本 · 已选')).toBeVisible()
  await plan.getByRole('button', { name: '取消选择当前结果' }).click()
  await expect(plan.getByRole('checkbox')).not.toBeChecked()
  await plan.getByRole('button', { name: '全选当前结果' }).click()
  await search.fill('missing-film')
  await plan.getByRole('button', { name: '查看全部文件' }).click()
  await expect(plan.getByRole('checkbox', { name: '整理 NORMAL-001' })).toBeChecked()
  await expect(plan.getByRole('checkbox', { name: '整理 EXTRA-002' })).not.toBeChecked()
  await expect(plan.getByRole('checkbox', { name: '整理 EXTRA-003' })).toBeChecked()
  await page.getByRole('button', { name: '确认整理（2 个文件）' }).click()
  expect(await page.evaluate(() => (window as any).appliedIds)).toEqual([0, 2])
})

test('a scan with over 300 unrecognized files allows finding and correcting the final file', async ({ page }) => {
  await overrideApi(page, `
    const settings = api.getSettings;
    api.getSettings = async () => ({ ...await settings(), source_dirs: ['/mock/source'] });
    const items = Array.from({ length: 305 }, (_, id) => ({ id, path: '/mock/source/clip-' + id + '.mp4', size: 12345, code: null, info: {} }));
    api.scanStart = async () => {
      const { emitMock } = await import('/src/lib/events.ts');
      emitMock('scan:items', { jobId: 10001, items });
      emitMock('scan:done', { jobId: 10001, total: items.length, unrecognized: items.length, codes: 0, complete: true, errors: [], skipped: [] });
      return 10001
    };
    api.setItemCode = async (id, text) => ({ ...items[id], code: { id: text, content_type: 'censored' }, localImports: [] });
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  const needsCode = page.getByRole('region', { name: '需要确认的文件' })
  await needsCode.getByRole('textbox').first().fill('DRAFT-001')
  await needsCode.getByRole('button', { name: '下一页文件' }).click()
  await expect(needsCode.getByText('clip-50.mp4', { exact: true })).toBeVisible()
  await needsCode.getByRole('searchbox').fill('clip-304.mp4')
  await expect(needsCode.getByRole('textbox')).toHaveCount(1)
  await needsCode.getByRole('textbox').fill('FIX-304')
  await needsCode.getByRole('button', { name: '确定', exact: true }).click()
  await expect(needsCode.getByText('没有匹配的文件')).toBeVisible()
  await needsCode.getByRole('button', { name: '清除搜索' }).click()
  await expect(needsCode.getByText('clip-0.mp4', { exact: true })).toBeVisible()
  await expect(needsCode.getByRole('textbox').first()).toHaveValue('DRAFT-001')
  await page.getByText(/查看已识别文件/).click()
  await expect(page.getByRole('button', { name: '修改番号 FIX-304' })).toBeVisible()
})
