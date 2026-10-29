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
