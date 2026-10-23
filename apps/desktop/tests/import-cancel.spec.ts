import { expect, test, type Page } from '@playwright/test'
import { seedLibrary } from './helpers'

type Kind = 'scrape' | 'url' | 'apply'

// Hold only the native start response; the real flow still owns events and UI state.
async function prepare(page: Page, kind: Kind) {
  await seedLibrary(page)
  await page.goto('/')
  await page.evaluate(async kind => {
    const { api } = await import('/src/lib/api.ts')
    const { flow } = await import('/src/lib/importflow.svelte.ts')
    const { emitMock } = await import('/src/lib/events.ts')
    await flow.init()
    flow.items = [{ id: 0, path: '/source/ABC-123.mp4', size: 1,
      info: { number: 'ABC-123', content_type: 'censored', prefix: 'ABC', cd: null,
        has_subtitle: false, mosaic: 'censored', definition: null, matched: true },
      code: { kind: 'Standard', id: 'ABC-123', content_type: 'censored', part: null,
        subtitled: false, mosaic: 'censored', definition: null } }]
    const state: any = { flow, cancelled: [], starts: [] }
    const start = () => new Promise<number>((resolve, reject) => state.starts.push({ resolve, reject }))
    api.scrapeStart = start
    api.scrapeUrlStart = start
    api.applyStart = start
    api.jobCancel = async jobId => { state.cancelled.push(jobId ?? null); return true }
    state.start = () => kind === 'apply' ? flow.startApply()
      : kind === 'url' ? flow.scrapeUrl('ABC-123', 'https://example.test/movie/ABC-123') : flow.startScrape()
    state.progress = (jobId: number) => {
      if (kind === 'apply') emitMock('apply:progress', { jobId, phase: 'images', done: 0, total: 1, failed: 0, code: null })
      else emitMock('scrape:event', { jobId, code: 'ABC-123', status: { kind: 'scraping', site: 'example' }, done: 0, total: 1, preview: null, note: null })
    }
    state.done = (jobId: number) => {
      if (kind === 'apply') emitMock('apply:done', { jobId, journalId: 'test', applied: 0, failed: [],
        cancelled: true, indexError: null, cleaned: [] })
      else emitMock('scrape:done', { jobId, found: 0, notFound: 0, failed: 0, cancelled: true })
    }
    state.pending = state.start()
    ;(window as any).cancelTest = state
  }, kind)
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
}

test('another task’s logs cannot claim a pending apply or receive its stop request', async ({ page }) => {
  await prepare(page, 'apply')
  await page.getByRole('button', { name: '停止', exact: true }).click()
  await page.evaluate(async () => {
    const { emitMock } = await import('/src/lib/events.ts')
    emitMock('log:line', { jobId: 910, time: '00:00:00', level: 'info', target: 'kiroku::pipeline', message: '另一补图任务' })
  })
  expect(await page.evaluate(() => (window as any).cancelTest.cancelled)).toEqual([])
  expect(await page.evaluate(() => (window as any).cancelTest.flow.logLines)).toEqual([])
  await page.evaluate(async () => {
    const state = (window as any).cancelTest
    state.starts[0].reject('已有任务在运行:repair')
    await state.pending
  })
  expect(await page.evaluate(() => (window as any).cancelTest.cancelled)).toEqual([])
})

test('early apply logs are retained only after an owned event confirms the job', async ({ page }) => {
  await prepare(page, 'apply')
  await page.evaluate(async () => {
    const { emitMock } = await import('/src/lib/events.ts')
    emitMock('log:line', { jobId: 910, time: '00:00:00', level: 'info', target: 'kiroku::pipeline', message: '其他任务' })
    emitMock('log:line', { jobId: 911, time: '00:00:01', level: 'info', target: 'kiroku::pipeline', message: '本次准备图片' })
    const state = (window as any).cancelTest
    state.progress(911)
    state.starts[0].resolve(911)
    await state.pending
  })
  expect(await page.evaluate(() => (window as any).cancelTest.flow.logLines.map((line: any) => line.message))).toEqual(['本次准备图片'])
  await page.getByRole('button', { name: '停止', exact: true }).click()
  expect(await page.evaluate(() => (window as any).cancelTest.cancelled)).toEqual([911])
})

for (const kind of ['scrape', 'url', 'apply'] as const) {
  test(`${kind}: stop before the start response cancels only the returned job`, async ({ page }) => {
    await prepare(page, kind)
    await page.getByRole('button', { name: '停止', exact: true }).click()
    expect(await page.evaluate(() => (window as any).cancelTest.cancelled)).toEqual([])
    await page.evaluate(async () => {
      const state = (window as any).cancelTest
      state.starts[0].resolve(901)
      await state.pending
    })
    expect(await page.evaluate(() => (window as any).cancelTest.cancelled)).toEqual([901])
    await page.evaluate(() => (window as any).cancelTest.done(901))
    expect(await page.evaluate(() => (window as any).cancelTest.flow.busy)).toBe(false)
    await page.evaluate(() => (window as any).cancelTest.flow.cancel())
    expect(await page.evaluate(() => (window as any).cancelTest.cancelled)).toEqual([901])
  })

  test(`${kind}: an early event delivers a pending stop before the start response`, async ({ page }) => {
    await prepare(page, kind)
    await page.getByRole('button', { name: '停止', exact: true }).click()
    await page.evaluate(() => (window as any).cancelTest.progress(902))
    expect(await page.evaluate(() => (window as any).cancelTest.cancelled)).toEqual([902])
    await page.evaluate(async () => {
      const state = (window as any).cancelTest
      state.starts[0].resolve(902)
      await state.pending
    })
    expect(await page.evaluate(() => (window as any).cancelTest.cancelled)).toEqual([902])
  })

  test(`${kind}: a rejected start never cancels the other task occupying the slot`, async ({ page }) => {
    await prepare(page, kind)
    await page.getByRole('button', { name: '停止', exact: true }).click()
    await page.evaluate(async () => {
      const state = (window as any).cancelTest
      state.starts[0].reject('已有任务在运行:repair')
      await state.pending
    })
    expect(await page.evaluate(() => (window as any).cancelTest.cancelled)).toEqual([])
    expect(await page.evaluate(() => ({ busy: (window as any).cancelTest.flow.busy,
      error: (window as any).cancelTest.flow.error }))).toEqual({ busy: false, error: '已有任务在运行:repair' })
  })

  test(`${kind}: a completed task's late start response cannot claim the next task`, async ({ page }) => {
    await prepare(page, kind)
    await page.evaluate(async () => {
      const state = (window as any).cancelTest
      state.done(903)
      state.next = state.start()
      state.starts[0].resolve(903)
      await state.pending
      state.progress(904)
      state.starts[1].resolve(904)
      await state.next
      await state.flow.cancel()
    })
    expect(await page.evaluate(() => (window as any).cancelTest.cancelled)).toEqual([904])
    await page.evaluate(() => (window as any).cancelTest.done(904))
    expect(await page.evaluate(() => (window as any).cancelTest.flow.busy)).toBe(false)
  })

  test(`${kind}: a completed task's late start error cannot fail the next task`, async ({ page }) => {
    await prepare(page, kind)
    await page.evaluate(async () => {
      const state = (window as any).cancelTest
      state.done(905)
      state.next = state.start()
      state.starts[0].reject('旧启动响应失败')
      await state.pending
      state.starts[1].resolve(906)
      await state.next
    })
    expect(await page.evaluate(() => ({ busy: (window as any).cancelTest.flow.busy,
      error: (window as any).cancelTest.flow.error }))).toEqual({ busy: true, error: null })
    await page.getByRole('button', { name: '停止', exact: true }).click()
    expect(await page.evaluate(() => (window as any).cancelTest.cancelled)).toEqual([906])
  })

  test(`${kind}: a failed cancellation leaves the task running and can be retried`, async ({ page }) => {
    await prepare(page, kind)
    await page.evaluate(async () => {
      const state = (window as any).cancelTest
      const { api } = await import('/src/lib/api.ts')
      api.jobCancel = async id => {
        state.cancelled.push(id)
        if (state.cancelled.length === 1) throw new Error('停止请求失败')
        return true
      }
      state.starts[0].resolve(907)
      await state.pending
    })
    await page.getByRole('button', { name: '停止', exact: true }).click()
    await expect(page.getByRole('alert')).toContainText('停止请求失败')
    expect(await page.evaluate(() => (window as any).cancelTest.flow.busy)).toBe(true)
    await page.getByRole('button', { name: '停止', exact: true }).click()
    expect(await page.evaluate(() => (window as any).cancelTest.cancelled)).toEqual([907, 907])
    await page.evaluate(() => (window as any).cancelTest.done(907))
    expect(await page.evaluate(() => (window as any).cancelTest.flow.busy)).toBe(false)
  })
}
