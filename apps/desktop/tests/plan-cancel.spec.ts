import { expect, test } from '@playwright/test'
import { seedLibrary } from './helpers'

test.beforeEach(async ({ page }) => { await seedLibrary(page); await page.goto('/') })

test('early planning cancellation waits for its matching job and discards late plans', async ({ page }) => {
  await page.evaluate(async () => {
    const { api } = await import('/src/lib/api.ts')
    const { flow } = await import('/src/lib/importflow.svelte.ts')
    const { emitMock } = await import('/src/lib/events.ts')
    await flow.init()
    flow.items = [{ id: 0, path: '/source/ABC-123.mp4', code: { id: 'ABC-123' } }] as any
    api.scrapeStart = async () => {
      emitMock('scrape:event', { jobId: 77, code: 'ABC-123', status: { kind: 'found' }, done: 1, total: 1 })
      emitMock('scrape:done', { jobId: 77, found: 1, notFound: 0, failed: 0, cancelled: false })
      return 77
    }
    await flow.startScrape()
    const state: any = { flow, emitMock, cancelled: [], requestId: null }
    ;(window as any).planningTest = state
    api.jobCancel = async id => { state.cancelled.push(id); return true }
    api.planBuild = async (_ids, requestId) => {
      state.requestId = requestId
      return new Promise(resolve => { state.finish = () => resolve([{ itemId: 0, code: 'ABC-123', conflict: null, selected: true } as any]) })
    }
    state.build = flow.buildPlan()
    await flow.cancel()
  })
  expect(await page.evaluate(() => (window as any).planningTest.cancelled)).toEqual([])
  await page.evaluate(() => {
    const state = (window as any).planningTest
    state.emitMock('plan:started', { jobId: 78, requestId: 'unrelated-attempt' })
  })
  expect(await page.evaluate(() => (window as any).planningTest.cancelled)).toEqual([])
  await page.evaluate(async () => {
    const state = (window as any).planningTest
    state.emitMock('plan:started', { jobId: 79, requestId: state.requestId })
    await Promise.resolve()
    state.finish()
    await state.build
  })
  expect(await page.evaluate(() => (window as any).planningTest.cancelled)).toEqual([79])
  expect(await page.evaluate(() => ({ plans: (window as any).planningTest.flow.plans, planning: (window as any).planningTest.flow.planning }))).toEqual({ plans: [], planning: false })
})

test('mock planning emits its own id, honours cancellation, and releases the job', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const { api } = await import('/src/lib/api.ts')
    const { listen } = await import('/src/lib/events.ts')
    const events: any[] = []
    const stop = await listen('plan:started', event => {
      events.push(event)
      if (event.requestId === 'cancel-this-plan') void api.jobCancel(event.jobId)
    })
    let error = ''
    try { await api.planBuild([], 'cancel-this-plan') } catch (e) { error = String(e) }
    const next = await api.planBuild([], 'next-plan')
    stop()
    return { error, next, events }
  })
  expect(result.error).toContain('取消')
  expect(result.next).toEqual([])
  expect(result.events.map(event => event.requestId)).toEqual(['cancel-this-plan', 'next-plan'])
  expect(result.events[1].jobId).toBeGreaterThan(result.events[0].jobId)
})

test('mock planning rejects a settings commit during preparation', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const { api } = await import('/src/lib/api.ts')
    const settings = await api.getSettings()
    const plan = api.planBuild([], 'changing-settings').then(() => '', error => String(error))
    await api.saveSettings({ ...settings, log: { ...settings.log, verbose: !settings.log.verbose } })
    return await plan
  })
  expect(result).toContain('设置或媒体库已变化')
})

test('planning offers a stop button while the backend is preparing', async ({ page }) => {
  await page.evaluate(async () => {
    const { api } = await import('/src/lib/api.ts')
    const { flow } = await import('/src/lib/importflow.svelte.ts')
    const { emitMock } = await import('/src/lib/events.ts')
    await flow.init()
    flow.step = 'scrape'
    flow.items = [{ id: 0, path: '/source/ABC-123.mp4', code: { id: 'ABC-123' } }] as any
    flow.statuses = { 'ABC-123': { kind: 'found' } }
    api.planBuild = async (_ids, requestId) => {
      emitMock('plan:started', { jobId: 91, requestId: requestId ?? null })
      return new Promise((_resolve, reject) => { (window as any).stopPlanning = () => reject('计划已取消') })
    }
    api.jobCancel = async id => { if (id === 91) (window as any).stopPlanning(); return id === 91 }
  })
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  await page.getByRole('button', { name: '停止', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('计划已取消')
  await expect(page.getByRole('button', { name: '预览整理', exact: true })).toBeEnabled()
})
