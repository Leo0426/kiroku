import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

test('a saved source folder that no longer exists explains why and can be removed', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    window.testApi = api;
    const missing = '/mock/gone-source';
    const reason = '文件夹不存在，可能已被移动、改名，或所在磁盘未连接';
    const s = await api.getSettings(); s.source_dirs = [...s.source_dirs, missing]; await api.saveSettings(s);
    const withIssue = read => async (...args) => {
      const r = await read(...args);
      const dirs = (await api.getSettings()).source_dirs;
      if (!dirs.includes(missing)) return r;
      return { ...r, dirs, complete: false, errors: [{ path: missing, message: reason }], errorsTotal: 1 };
    };
    api.pendingSummary = withIssue(api.pendingSummary);
    api.pendingList = withIssue(api.pendingList);
  `)
  await page.goto('/')
  const alert = page.getByRole('alert').filter({ hasText: '待整理文件夹不可用' })
  await expect(alert).toContainText('/mock/gone-source')
  await expect(alert).toContainText('文件夹不存在')
  await alert.getByRole('button', { name: '从待整理中移除', exact: true }).click()
  await expect(alert).toHaveCount(0)
  expect(await page.evaluate(async () => (await (window as any).testApi.getSettings()).source_dirs)).not.toContain('/mock/gone-source')
})

test('unreadable children leave their source available and a refresh clears the partial result', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    const settings = await api.getSettings();
    settings.source_dirs = ['/mock/source'];
    await api.saveSettings(settings);
    const withIssues = result => ({ ...result, complete: false, errorsTotal: 179,
      errors: Array.from({ length: 100 }, (_, i) => ({ path: '/mock/source/folder-' + i, message: '找不到文件或目录' })) });
    const count = api.pendingSummary, preview = api.pendingList;
    api.pendingSummary = async (...args) => withIssues(await count(...args));
    api.pendingList = async (id, force) => {
      const result = await preview(id, force);
      return force ? result : withIssues(result);
    };
  `)
  await page.goto('/')
  const notice = page.getByRole('alert', { name: '待整理扫描结果不完整' })
  await expect(notice).toContainText('179 处')
  await expect(notice).toContainText('/mock/source/folder-0')
  await expect(page.getByRole('alert', { name: '待整理文件夹不可用' })).toHaveCount(0)
  await expect(notice.getByRole('button', { name: '从待整理中移除' })).toHaveCount(0)
  await notice.getByRole('button', { name: '查看待整理' }).click()
  await expect(page.locator('.pending').getByRole('alert')).toContainText('/mock/source/folder-0')
  await expect(page.getByRole('button', { name: '开始整理', exact: true })).toBeEnabled()
  await page.getByRole('button', { name: '刷新待整理', exact: true }).click()
  await expect(page.locator('.pending').getByRole('alert')).toHaveCount(0)
  await page.getByRole('tab', { name: /已整理/ }).click()
  await expect(notice).toHaveCount(0)
})
