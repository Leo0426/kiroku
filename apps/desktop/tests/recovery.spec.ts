import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

test.beforeEach(async ({ page }) => seedLibrary(page))

test('rebuild failure is shown as an error and permits retry', async ({ page }) => {
  await page.route('**/src/lib/events.ts*', async route => {
    const response = await route.fetch()
    await route.fulfill({ response, body: (await response.text()) + '\nwindow.emitMockEvent = emitMock;' })
  })
  await overrideApi(page, `api.libraryRebuild = async () => { window.emitMockEvent('library:rebuilt', { jobId: 123, root: '/mock/library', count: 0, cancelled: false, error: 'database unavailable' }); return 123; };`)
  await page.goto('/')
  await page.getByRole('button', { name: '设置', exact: true }).click()
  await page.getByRole('button', { name: '重建索引', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('database unavailable')
  await expect(page.getByRole('button', { name: '重建索引', exact: true })).toBeEnabled()
  await expect(page.getByText('索引已重建:0 部影片', { exact: true })).toHaveCount(0)
})

test('damaged journal remains visible with a diagnosis instead of disappearing', async ({ page }) => {
  await overrideApi(page, `api.historyList = async () => ({ items: [{ id: '123', ts: 123, status: 'partial', itemCount: 0, opCount: 0, codes: [], trashed: 0, pendingOperations: 0, error: 'invalid journal JSON' }], total: 1, next: null });`)
  await page.goto('/')
  await page.getByRole('button', { name: '历史', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('invalid journal JSON')
  await expect(page.getByRole('button', { name: '撤销', exact: true })).toHaveCount(0)
  await expect(page.getByText('还没有操作记录')).toHaveCount(0)
})

test('startup failure offers retry without a blank screen', async ({ page }) => {
  await overrideApi(page, `
    const getSettings = api.getSettings;
    let failed = false;
    api.getSettings = async () => {
      if (!failed) { failed = true; throw new Error('settings unavailable'); }
      return getSettings();
    };
  `)
  await page.goto('/')
  await expect(page.getByRole('alert')).toContainText('settings unavailable')
  await page.getByRole('button', { name: '重试' }).click()
  await expect(page.getByRole('navigation', { name: '主导航' })).toBeVisible()
})

test('library failure is recoverable and is not shown as an empty library', async ({ page }) => {
  await overrideApi(page, `
    const libraryList = api.libraryList;
    let failed = false;
    api.libraryList = async (...args) => {
      if (!failed) { failed = true; throw new Error('library unavailable'); }
      return libraryList(...args);
    };
  `)
  await page.goto('/')
  await expect(page.getByRole('alert')).toContainText('library unavailable')
  await expect(page.getByText('媒体库还是空的')).toHaveCount(0)
  await page.getByRole('button', { name: '重试' }).click()
  await expect(page.locator('button.pcard').first()).toBeVisible()
})

test('detail failure offers retry and eventually displays the movie', async ({ page }) => {
  await overrideApi(page, `
    const libraryGet = api.libraryGet;
    let failed = false;
    api.libraryGet = async (...args) => {
      if (!failed) { failed = true; throw new Error('detail unavailable'); }
      return libraryGet(...args);
    };
  `)
  await page.goto('/')
  await page.locator('button.pcard').first().click()
  await expect(page.getByRole('alert')).toContainText('detail unavailable')
  await page.getByRole('button', { name: '重试' }).click()
  await expect(page.getByRole('button', { name: '在文件夹中显示' })).toBeVisible()
})

test('history failure is not confused with no history and can be retried', async ({ page }) => {
  await overrideApi(page, `
    const historyList = api.historyList;
    api.historyList = async (id, page) => {
      if (window.failHistoryRead) { window.failHistoryRead = false; throw new Error('history unavailable'); }
      const result = await historyList(id, page);
      window.initialHistoryReadFinished = true;
      return result;
    };
  `)
  await page.goto('/')
  // Recovery uses its fresh intake path; the display-list failure belongs to this page.
  await page.evaluate(() => { (window as unknown as { failHistoryRead: boolean }).failHistoryRead = true })
  await page.getByRole('button', { name: '历史', exact: true }).click()
  await expect(page.locator('main').getByRole('alert')).toContainText('history unavailable')
  await expect(page.getByText('还没有整理记录')).toHaveCount(0)
  await page.getByRole('button', { name: '重试' }).click()
  await expect(page.getByText('还没有操作记录')).toBeVisible()
})

test('unsaved settings survive cancelled navigation and saved values survive leaving', async ({ page }) => {
  await page.goto('/')
  await page.getByRole('button', { name: '设置', exact: true }).click()
  if (await page.getByRole('tab', { name: '命名与 NFO', exact: true }).count()) await page.getByRole('tab', { name: '命名与 NFO', exact: true }).click()
  await page.getByLabel('目录模板').fill('{code}/{title}')
  await page.getByRole('button', { name: '媒体库', exact: true }).click()
  const dialog = page.getByRole('alertdialog')
  await expect(dialog).toBeVisible()
  await dialog.getByRole('button', { name: '继续编辑' }).click()
  await expect(page.getByLabel('目录模板')).toHaveValue('{code}/{title}')
  await page.getByRole('button', { name: /保存更改/ }).click()
  await expect(page.getByRole('button', { name: '已保存', exact: true })).toBeVisible()
  await page.getByRole('button', { name: '媒体库', exact: true }).click()
  await expect(dialog).toHaveCount(0)
  await page.getByRole('button', { name: '设置', exact: true }).click()
  if (await page.getByRole('tab', { name: '命名与 NFO', exact: true }).count()) await page.getByRole('tab', { name: '命名与 NFO', exact: true }).click()
  await expect(page.getByLabel('目录模板')).toHaveValue('{code}/{title}')
})

test('explicit discard leaves settings and restores the stored values', async ({ page }) => {
  await page.goto('/')
  await page.getByRole('button', { name: '设置', exact: true }).click()
  if (await page.getByRole('tab', { name: '命名与 NFO', exact: true }).count()) await page.getByRole('tab', { name: '命名与 NFO', exact: true }).click()
  await page.getByLabel('目录模板').fill('{code}/{title}')
  await page.getByRole('button', { name: '历史', exact: true }).click()
  await page.getByRole('alertdialog').getByRole('button', { name: '放弃更改并离开' }).click()
  await expect(page.getByRole('heading', { name: '历史', exact: true })).toBeVisible()
  await page.getByRole('button', { name: '设置', exact: true }).click()
  if (await page.getByRole('tab', { name: '命名与 NFO', exact: true }).count()) await page.getByRole('tab', { name: '命名与 NFO', exact: true }).click()
  await expect(page.getByLabel('目录模板')).toHaveValue('{actor}/{code}')
})

test('settings load failure can be retried', async ({ page }) => {
  await overrideApi(page, `
    const getSettings = api.getSettings;
    let calls = 0;
    api.getSettings = async () => {
      if (++calls === 2) throw new Error('settings read failed');
      return getSettings();
    };
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '设置', exact: true }).click()
  if (await page.getByRole('tab', { name: '命名与 NFO', exact: true }).count()) await page.getByRole('tab', { name: '命名与 NFO', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('settings read failed')
  await page.getByRole('button', { name: '重试' }).click()
  await page.getByRole('tab', { name: '命名与 NFO', exact: true }).click()
  await expect(page.getByLabel('目录模板')).toHaveValue('{actor}/{code}')
})

test('edits made while saving remain unsaved until another save', async ({ page }) => {
  await overrideApi(page, `
    const saveSettings = api.saveSettings;
    api.saveSettings = async (settings) => {
      await new Promise(resolve => { window.finishSave = resolve; });
      return saveSettings(settings);
    };
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '设置', exact: true }).click()
  if (await page.getByRole('tab', { name: '命名与 NFO', exact: true }).count()) await page.getByRole('tab', { name: '命名与 NFO', exact: true }).click()
  await page.getByLabel('目录模板').fill('{code}')
  await page.getByRole('button', { name: /保存更改/ }).click()
  await expect(page.getByRole('button', { name: '保存中…' })).toBeDisabled()
  await page.getByLabel('目录模板').fill('{code}/{title}')
  await page.getByRole('button', { name: '历史', exact: true }).click()
  await expect(page.getByLabel('目录模板')).toBeVisible()
  await page.evaluate(() => (window as unknown as { finishSave: () => void }).finishSave())
  await expect(page.getByRole('button', { name: /保存更改/ })).toBeEnabled()
  await page.getByRole('button', { name: '历史', exact: true }).click()
  await page.getByRole('alertdialog').getByRole('button', { name: '放弃更改并离开' }).click()
  await page.getByRole('button', { name: '设置', exact: true }).click()
  if (await page.getByRole('tab', { name: '命名与 NFO', exact: true }).count()) await page.getByRole('tab', { name: '命名与 NFO', exact: true }).click()
  await expect(page.getByLabel('目录模板')).toHaveValue('{code}')
})

test('retrying failed event setup restores scan completion listeners', async ({ page }) => {
  await overrideApi(page, `api.pickDirectory = async () => '/mock/source';`)
  await page.route('**/src/lib/events.ts*', async (route) => {
    const response = await route.fetch()
    await route.fulfill({ response, body: (await response.text()) + `
      const originalListen = listen;
      let failed = false;
      listen = async (event, handler) => {
        if (event === 'scan:done' && !failed) { failed = true; throw new Error('event setup failed'); }
        return originalListen(event, handler);
      };
    ` })
  })
  await page.goto('/')
  await expect(page.getByRole('alert')).toContainText('event setup failed')
  await page.getByRole('button', { name: '重试' }).click()
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '添加文件夹', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await expect(page.getByRole('button', { name: /查找影片信息/ })).toBeEnabled()
})

test('details repair missing artwork and show its reversible journal', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `const list = api.libraryList; api.libraryList = async (q) => list({ ...q, text: 'ABQ-100' });`)
  await page.goto('/')
  await page.locator('button.pcard').first().click()
  await page.getByRole('button', { name: '补缺图', exact: true }).click()
  await page.getByRole('button', { name: '停止补图', exact: true }).click()
  await expect(page.getByRole('status', { name: '补图结果' })).toContainText('补图已停止：写入 0 张图片')
  await expect(page.getByLabel('影片详情').locator('.poster img')).toHaveCount(0)
  await page.getByRole('button', { name: '补缺图', exact: true }).click()
  await expect(page.getByRole('status', { name: '补图结果' })).toContainText('补图完成：写入 1 张图片')
  await expect(page.getByLabel('影片详情').locator('.poster img')).toBeVisible()
  await page.getByRole('button', { name: '撤销本次补图', exact: true }).click()
  await expect(page.getByLabel('影片详情').locator('.poster img')).toHaveCount(0)
  await expect(page.getByLabel('影片详情')).toBeVisible()
})

test('repair ignores stale completion received before invoke resolves', async ({ page }) => {
  await page.route('**/src/lib/events.ts*', async route => {
    const response = await route.fetch()
    await route.fulfill({ response, body: (await response.text()) + '\nwindow.emitMockEvent = emitMock;' })
  })
  await overrideApi(page, `
    api.repairImagesStart = async (code) => {
      const common = { root: '/mock/library', code, cancelled: false, journalId: null, indexError: null, warnings: [] };
      window.emitMockEvent('repair:done', { ...common, jobId: 80, written: 99, failed: [] });
      window.emitMockEvent('repair:done', { ...common, jobId: 81, written: 0, failed: [[code, '候选图片全部无效']] });
      return 81;
    };
  `)
  await page.goto('/')
  await page.locator('button.pcard').first().click()
  await page.getByRole('button', { name: '补缺图', exact: true }).click()
  await expect(page.getByRole('status', { name: '补图结果' })).toContainText('候选图片全部无效')
  await expect(page.getByRole('status', { name: '补图结果' })).not.toContainText('99 张')
})
