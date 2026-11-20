import { expect, test, type Page } from '@playwright/test'
import { overrideApi } from './helpers'

const nav = (page: Page, name: string) => page.getByRole('navigation', { name: '主导航' }).getByRole('button', { name, exact: true })

async function finishImport(page: Page) {
  await page.addInitScript(() => localStorage.setItem('kiroku-mock-settings-v2', JSON.stringify({
    library_root: '/mock/library', source_dirs: ['/mock/companion-import/SMPL-421.mp4'],
  })))
  await page.goto('/')
  await nav(page, '导入整理').click()
  await page.getByRole('button', { name: '开始扫描' }).click()
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  await page.getByRole('button', { name: /确认整理/ }).click()
  await expect(page.getByRole('button', { name: '撤销本批' })).toBeEnabled()
}

for (const where of ['完成页', '历史']) {
  test('从' + where + '撤销后完成页仍能认出本批', async ({ page }) => {
    await finishImport(page)
    if (where === '历史') {
      await nav(page, '历史').click()
      await page.getByRole('button', { name: '撤销', exact: true }).click()
    } else {
      await page.getByRole('button', { name: '撤销本批' }).click()
    }
    await page.getByRole('button', { name: '确认撤销' }).click()
    if (where === '历史') await expect(page.getByRole('list', { name: '操作记录' }).getByText('已撤销')).toBeVisible()
    else await expect(page.getByText('本批已撤销', { exact: true })).toBeVisible()
    await nav(page, '媒体库').click()
    await nav(page, '导入整理').click()
    await expect(page.getByText('本批已撤销', { exact: true })).toBeVisible()
    await expect(page.getByRole('button', { name: '撤销本批' })).toHaveCount(0)
  })
}

test('an unfinished rollback stays busy across pages and allows a single retry', async ({ page }) => {
  await overrideApi(page, `
    const rollback = api.rollback;
    window.undoAttempts = 0;
    api.rollback = async (...args) => {
      if (++window.undoAttempts === 1) {
        await new Promise(resolve => { window.finishUndo = resolve });
        return { status: 'partial', failures: [], indexError: null };
      }
      return rollback(...args);
    };
  `)
  await finishImport(page)
  await page.getByRole('button', { name: '撤销本批' }).click()
  await page.getByRole('button', { name: '确认撤销' }).click()
  await expect(page.getByRole('button', { name: '撤销中…' })).toBeDisabled()
  await nav(page, '历史').click()
  await expect(page.getByRole('button', { name: '撤销中…' })).toBeDisabled()
  await nav(page, '导入整理').click()
  await expect(page.getByRole('button', { name: '再来一批' })).toBeDisabled()
  await page.evaluate(() => (window as any).finishUndo())
  await expect(page.getByRole('alert')).toContainText('尚有操作未撤销')
  await expect(page.getByText('本批已撤销', { exact: true })).toHaveCount(0)
  await nav(page, '历史').click()
  await page.getByRole('button', { name: '撤销', exact: true }).click()
  await page.getByRole('button', { name: '确认撤销' }).click()
  await expect(page.getByRole('list', { name: '操作记录' }).getByText('已撤销')).toBeVisible()
  expect(await page.evaluate(() => (window as any).undoAttempts)).toBe(2)
})

test('switching libraries invalidates the confirmation and checks the root at dispatch', async ({ page }) => {
  await finishImport(page)
  await page.getByRole('button', { name: '撤销本批' }).click()
  const stale = await page.evaluate(async () => {
    const { app } = await import('/src/lib/state.svelte.ts')
    const { api } = await import('/src/lib/api.ts')
    const records = await api.historyList(crypto.randomUUID())
    const id = records.items[0].id
    await app.setLibraryRoot('/mock/second')
    try { await api.rollback(id, '/mock/library') }
    catch (error) { return String(error) }
  })
  expect(stale).toContain('媒体库已变化')
  await expect(page.getByRole('button', { name: '确认撤销' })).toHaveCount(0)
  await expect(page.getByRole('button', { name: '撤销本批' })).toHaveCount(0)
  await page.getByRole('button', { name: '前往媒体库设置' }).click()
  await expect(page.locator('#library-location')).toBeFocused()
  await nav(page, '导入整理').click()
  await page.evaluate(async () => {
    const { app } = await import('/src/lib/state.svelte.ts')
    await app.setLibraryRoot('/mock/library')
  })
  await expect(page.getByRole('button', { name: '撤销本批' })).toBeEnabled()
  await expect(page.getByRole('button', { name: '确认撤销' })).toHaveCount(0)
})

for (const indexOnly of [false, true]) {
  test('the outcome keeps the ' + (indexOnly ? 'index warning after a completed rollback' : 'rollback failure and retry') + ' across visits', async ({ page }) => {
    const issue = indexOnly ? '示例索引不可写' : '示例文件的原位置已被占用'
    const second = '第二个示例文件已被修改'
    await overrideApi(page, `
      const rollback = api.rollback;
      let first = true;
      api.rollback = async (...args) => {
        if (!first) return rollback(...args);
        first = false;
        ${indexOnly
          ? 'return { ...await rollback(...args), indexError: ' + JSON.stringify(issue) + ' };'
          : 'return { status: "partial", failures: ' + JSON.stringify([issue, second]) + ', indexError: null };'}
      };
    `)
    await finishImport(page)
    await page.getByRole('button', { name: '撤销本批' }).click()
    await page.getByRole('button', { name: '确认撤销' }).click()
    if (!indexOnly) {
      const toast = page.getByRole('alert').filter({ hasText: '共 2 项未撤销' })
      await expect(toast).toContainText(issue)
      await expect(toast).not.toContainText(second)
    }
    await nav(page, '历史').click()
    await nav(page, '导入整理').click()
    const outcome = page.getByRole('region', { name: '本批整理结果' })
    await expect(outcome.getByRole('heading', { name: indexOnly ? '本批已撤销' : '本批撤销未完成' })).toBeVisible()
    await expect(outcome).toContainText(issue)
    if (indexOnly) {
      await expect(outcome.getByRole('button', { name: '前往索引设置' })).toBeVisible()
      await expect(page.getByRole('button', { name: '重试撤销' })).toHaveCount(0)
    } else {
      await outcome.getByRole('button', { name: '查看全部 2 条错误' }).click()
      await expect(outcome).toContainText(second)
      await page.getByRole('button', { name: '重试撤销' }).click()
      await page.getByRole('button', { name: '确认撤销' }).click()
      await expect(outcome.getByRole('heading', { name: '本批已撤销' })).toBeVisible()
      await expect(outcome).not.toContainText(issue)
    }
  })
}
