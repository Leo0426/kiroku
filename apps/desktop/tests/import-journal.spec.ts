import { expect, test, type Page } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

async function finishBatch(page: Page, setup = '') {
  await seedLibrary(page)
  await overrideApi(page, `
    const getSettings = api.getSettings
    api.getSettings = async () => ({ ...(await getSettings()), source_dirs: ['/mock/inbox'] })
    const scrapeStart = api.scrapeStart
    api.scrapeStart = ids => scrapeStart(ids.filter(id => id === 3))
    ${setup}
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描' }).click()
  await page.getByRole('button', { name: /查找影片信息/ }).click()
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  await page.getByRole('button', { name: /确认整理/ }).click()
  await expect(page.getByRole('heading', { name: '整理完成' })).toBeVisible()
}

test('从完成页展开本批实际操作并在撤销后看到对应状态', async ({ page }) => {
  await finishBatch(page)
  await page.getByRole('button', { name: '查看本批操作' }).click()
  const detail = page.getByRole('region', { name: '历史操作详情' })
  await expect(detail.getByRole('listitem').first()).toContainText('日志已记录')
  await page.getByRole('button', { name: '撤销本批' }).click()
  await page.getByRole('button', { name: '确认撤销' }).click()
  await expect(detail.getByRole('listitem').first()).toContainText('已撤销')
  await page.getByRole('button', { name: '再来一批' }).click()
  await expect(detail).toHaveCount(0)
})

test('本批详情翻页失败后，重试继续读失败的那一页', async ({ page }) => {
  await finishBatch(page, `
    const read = api.historyDetail
    let firstAttempt = true
    api.historyDetail = async (request, id, offset, root) => {
      const page = await read(request, id, offset, root)
      if (offset === 100 && firstAttempt) { firstAttempt = false; throw '临时读取失败' }
      return { ...page, total: 101, next: offset ? null : 100,
        operations: Array.from({ length: offset ? 1 : 100 }, (_, i) => ({ index: offset+i, state: 'recorded', description: '文件操作 ' + (offset+i+1) })) }
    }
  `)
  await page.getByRole('button', { name: '查看本批操作' }).click()
  const detail = page.getByRole('region', { name: '历史操作详情' })
  await detail.getByRole('button', { name: '下一页操作' }).click()
  await expect(detail.getByRole('alert')).toContainText('临时读取失败')
  await detail.getByRole('button', { name: '重试详情' }).click()
  await expect(detail.getByRole('listitem')).toHaveCount(1)
  await expect(detail.getByRole('listitem')).toContainText('文件操作 101')
  await detail.getByRole('button', { name: '上一页操作' }).click()
  await expect(detail.getByRole('listitem')).toHaveCount(100)
})

test('切库使本批详情停止读取，旧响应不会出现在新库', async ({ page }) => {
  await finishBatch(page, `
    const read = api.historyDetail
    api.historyDetail = async (...args) => {
      const data = await read(...args)
      return new Promise(resolve => { window.releaseBatchRead = () => resolve(data) })
    }
  `)
  await page.getByRole('button', { name: '查看本批操作' }).click()
  await page.waitForFunction(() => 'releaseBatchRead' in window)
  await page.evaluate(async () => {
    const { app } = await import('/src/lib/state.svelte.ts')
    await app.setLibraryRoot('/mock/second')
    ;(window as unknown as { releaseBatchRead(): void }).releaseBatchRead()
  })
  await expect(page.getByRole('region', { name: '历史操作详情' })).toHaveCount(0)
  await expect(page.getByText(/切回媒体库.*可查看本批操作/)).toContainText('/mock/library')
  await expect(page.getByRole('button', { name: '查看本批操作' })).toHaveCount(0)
})

test('本批日志没有文件操作时说明空结果', async ({ page }) => {
  await finishBatch(page, `
    const read = api.historyDetail
    api.historyDetail = async (...args) => ({ ...(await read(...args)), total: 0, next: null, operations: [] })
  `)
  await page.getByRole('button', { name: '查看本批操作' }).click()
  const detail = page.getByRole('region', { name: '历史操作详情' })
  await expect(detail).toContainText('此日志未记录文件操作。')
  await expect(detail.getByRole('button', { name: '下一页操作' })).toHaveCount(0)
})

test('撤销后的详情刷新失败时清除上一次的状态，重试后展示已撤销记录', async ({ page }) => {
  await finishBatch(page, `
    const read = api.historyDetail
    api.historyDetail = async (...args) => {
      const detail = await read(...args)
      if (detail.status === 'rolledBack' && !window.allowUpdatedDetail) throw '更新后的日志暂时不可读'
      return detail
    }
  `)
  await page.getByRole('button', { name: '查看本批操作' }).click()
  const detail = page.getByRole('region', { name: '历史操作详情' })
  await expect(detail.getByRole('listitem').first()).toContainText('日志已记录')
  await page.getByRole('button', { name: '撤销本批' }).click()
  await page.getByRole('button', { name: '确认撤销' }).click()
  await expect(detail.getByRole('alert')).toContainText('更新后的日志暂时不可读')
  await expect(detail.getByRole('listitem')).toHaveCount(0)
  await page.evaluate(() => { Object.assign(window, { allowUpdatedDetail: true }) })
  await detail.getByRole('button', { name: '重试详情' }).click()
  await expect(detail.getByRole('listitem').first()).toContainText('已撤销')
})

test('窄屏可用键盘阅读长日志并在原地重试长路径错误', async ({ page, browserName }) => {
  const tab = browserName === 'webkit' && process.platform === 'darwin' ? 'Alt+Tab' : 'Tab'
  await page.setViewportSize({ width: 360, height: 760 })
  await finishBatch(page, `
    const read = api.historyDetail
    const longPath = '/mock/' + '0123456789abcdef'.repeat(15) + '.nfo'
    let first = true
    api.historyDetail = async (...args) => {
      const result = await read(...args)
      if (first) { first = false; throw '无法读取 ' + longPath }
      const offset = args[2]
      return { ...result, total: 200, next: offset === 0 ? 100 : null,
        operations: Array.from({ length: 100 }, (_, i) => ({ index: offset+i, state: 'recorded', description: (offset+i+1) + ' ' + longPath })) }
    }
  `)
  const toggle = page.getByRole('button', { name: '查看本批操作' })
  await toggle.press('Enter')
  const detail = page.getByRole('region', { name: '历史操作详情' })
  const message = detail.getByRole('alert')
  await expect(message).toContainText('无法读取 /mock/')
  const overflows = (element: Element) => element.scrollWidth > element.clientWidth + 1
  expect(await message.evaluate(overflows)).toBe(false)
  expect(await page.locator('.view-shell').evaluate(overflows)).toBe(false)
  await page.keyboard.press(tab)
  await expect(detail.getByRole('button', { name: '重试详情' })).toBeFocused()
  await page.keyboard.press('Enter')
  const list = detail.getByRole('list', { name: '本页文件操作' })
  await expect(list.getByRole('listitem')).toHaveCount(100)
  await page.getByRole('button', { name: '收起本批操作' }).press(tab)
  await expect(list).toBeFocused()
  await page.keyboard.press('PageDown')
  await expect.poll(() => list.evaluate(el => el.scrollTop)).toBeGreaterThan(0)
  expect(await list.evaluate(overflows)).toBe(false)
  await page.keyboard.press(tab)
  await expect(detail.getByRole('button', { name: '下一页操作' })).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(list.getByRole('listitem').first()).toContainText('101 /mock/')
  await expect.poll(() => list.evaluate(el => el.scrollTop)).toBe(0)
})
