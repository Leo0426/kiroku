import { expect, test, type Page } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

async function prepare(page: Page, origin: 'details' | 'health' = 'details') {
  await seedLibrary(page)
  await overrideApi(page, `
    const s=window.repairTask={requests:[],cancels:[],cancelMode:'fail',pending:[],root:'/mock/library'};
    api.repairImagesStart=code=>new Promise((resolve,reject)=>s.requests.push({code,resolve,reject,root:s.root}));
    api.jobCancel=id=>{
      s.cancels.push(id??null);
      if(s.cancelMode==='hold')return new Promise((resolve,reject)=>s.pending.push({resolve,reject}));
      return s.cancelMode==='fail'?Promise.reject('停止请求失败'):Promise.resolve(true);
    };
    api.libraryHealth=async()=>({root:'/mock/library',scannedAt:1780000000,scanned:1,cancelled:false,issues:[{kind:'missingArtwork',severity:'warning',dir:'/mock/library/ABQ-100',code:'ABQ-100',path:'/mock/library/ABQ-100/poster.jpg',message:'缺少 poster 图片',action:'repairImages'}]});
    const {emitMock}=await import('/src/lib/events.ts');
    s.done=(index,fields={})=>{
      const r=s.requests[index];
      emitMock('repair:done',{jobId:901+index,root:r.root,code:r.code,written:1,failed:[],warnings:[],cancelled:true,journalId:'repair-test-'+index,indexError:null,...fields});
    };
  `)
  await page.goto('/')
  if (origin === 'health') {
    await page.getByRole('button', { name: '媒体库工具', exact: true }).click()
    await page.getByRole('button', { name: '检查媒体库', exact: true }).click()
    return page.getByLabel('媒体库健康检查', { exact: true })
  }
  await page.locator('button.pcard').first().click()
  const panel = page.getByLabel('影片详情', { exact: true })
  await expect(panel).toHaveClass(/armed/)
  return panel
}

const cancels = (page: Page) => page.evaluate(() => (window as any).repairTask.cancels)
const requestCount = (page: Page) => page.evaluate(() => (window as any).repairTask.requests.length)
const cancelAlert = (page: Page) => page.getByRole('alert').filter({ hasText: '停止请求失败' })

for (const origin of ['details', 'health'] as const) {
  test(`${origin}: early repair cancellation failure retains the running task and final result`, async ({ page }) => {
    const panel = await prepare(page, origin)
    await panel.getByRole('button', { name: '补缺图', exact: true }).click()
    await expect.poll(() => requestCount(page)).toBe(1)
    await panel.getByRole('button', { name: '停止补图', exact: true }).click()
    expect(await cancels(page)).toEqual([])
    await page.evaluate(() => (window as any).repairTask.requests[0].resolve(901))
    await expect(cancelAlert(page)).toBeVisible()
    await expect(panel.getByRole('button', { name: '停止补图', exact: true })).toBeVisible()
    await page.evaluate(() => (window as any).repairTask.done(0, { indexError: '索引暂不可用' }))
    await expect(panel.getByRole('button', { name: '停止补图', exact: true })).toHaveCount(0)
    await expect(panel.getByText(/补图已停止：写入 1 张图片/)).toBeVisible()
    await expect(panel.getByText(/索引.*索引暂不可用/)).toBeVisible()
    if (origin === 'details') await expect(panel.getByRole('button', { name: '撤销本次补图', exact: true })).toBeVisible()
    await expect(cancelAlert(page)).toHaveCount(0)
  })
}

test('repair stop failures are retryable and do not release the running task', async ({ page }) => {
  const panel = await prepare(page)
  await panel.getByRole('button', { name: '补缺图', exact: true }).click()
  await expect.poll(() => requestCount(page)).toBe(1)
  await page.evaluate(() => (window as any).repairTask.requests[0].resolve(901))
  await panel.getByRole('button', { name: '停止补图', exact: true }).click()
  await expect(cancelAlert(page)).toBeVisible()
  await cancelAlert(page).getByRole('button', { name: '重试停止', exact: true }).click()
  await expect.poll(() => cancels(page)).toEqual([901, 901])
  await expect(cancelAlert(page)).toHaveCount(1)
  await page.evaluate(() => (window as any).repairTask.cancelMode = 'ok')
  await cancelAlert(page).getByRole('button', { name: '重试停止', exact: true }).click()
  await expect.poll(() => cancels(page)).toEqual([901, 901, 901])
  await expect(cancelAlert(page)).toHaveCount(0)
  await expect(panel.getByRole('button', { name: '正在补图…', exact: true })).toBeDisabled()
  await page.evaluate(() => (window as any).repairTask.done(0))
  await expect(panel.getByRole('button', { name: '补缺图', exact: true })).toBeEnabled()
})

test('an early repair result settles before sending a queued cancellation', async ({ page }) => {
  const panel = await prepare(page)
  await panel.getByRole('button', { name: '补缺图', exact: true }).click()
  await expect.poll(() => requestCount(page)).toBe(1)
  await panel.getByRole('button', { name: '停止补图', exact: true }).click()
  await page.evaluate(() => {
    const s = (window as any).repairTask
    s.done(0, { jobId: 900, written: 99 })
    s.done(0)
    s.requests[0].resolve(901)
  })
  await expect(panel.getByRole('status', { name: '补图结果' })).toContainText('写入 1 张图片')
  expect(await cancels(page)).toEqual([])
  await expect(cancelAlert(page)).toHaveCount(0)
})

test('pending repair cancellation is deduplicated and its late failure cannot affect the next repair', async ({ page }) => {
  const panel = await prepare(page)
  await panel.getByRole('button', { name: '补缺图', exact: true }).click()
  await expect.poll(() => requestCount(page)).toBe(1)
  await page.evaluate(() => {
    const s = (window as any).repairTask
    s.cancelMode = 'hold'
    s.requests[0].resolve(901)
  })
  const stop = panel.getByRole('button', { name: '停止补图', exact: true })
  await stop.click()
  await stop.click()
  expect(await cancels(page)).toEqual([901])
  await page.evaluate(() => (window as any).repairTask.done(0))
  await expect(panel.getByRole('button', { name: '补缺图', exact: true })).toBeEnabled()
  await panel.getByRole('button', { name: '补缺图', exact: true }).click()
  await expect.poll(() => requestCount(page)).toBe(2)
  await page.evaluate(() => {
    const s = (window as any).repairTask
    s.requests[1].resolve(902)
    s.pending[0].reject('旧停止请求失败')
    s.done(0)
  })
  await expect(stop).toBeVisible()
  await expect(page.getByRole('alert').filter({ hasText: '旧停止请求失败' })).toHaveCount(0)
  await page.evaluate(() => (window as any).repairTask.done(1))
  await expect(stop).toHaveCount(0)
})

test('repair cancellation retry survives leaving details and changing the current library', async ({ page }) => {
  const panel = await prepare(page)
  await panel.getByRole('button', { name: '补缺图', exact: true }).click()
  await expect.poll(() => requestCount(page)).toBe(1)
  await page.evaluate(() => {
    const s = (window as any).repairTask
    s.cancelMode = 'hold'
    s.requests[0].resolve(901)
  })
  await panel.getByRole('button', { name: '停止补图', exact: true }).click()
  await panel.getByRole('button', { name: '关闭详情', exact: true }).click()
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.evaluate(async () => {
    const { app } = await import('/src/lib/state.svelte.ts')
    app.settings = { ...app.settings!, library_root: '/mock/other-library' }
    ;(window as any).repairTask.pending[0].reject('停止请求失败')
  })
  await expect(cancelAlert(page)).toBeVisible()
  await page.clock.install()
  await page.clock.fastForward(6000)
  await expect(cancelAlert(page)).toBeVisible()
  await page.evaluate(() => (window as any).repairTask.cancelMode = 'ok')
  await cancelAlert(page).getByRole('button', { name: '重试停止', exact: true }).click()
  expect(await cancels(page)).toEqual([901, 901])
  const version = await page.evaluate(async () => (await import('/src/lib/state.svelte.ts')).app.libraryVersion)
  await page.evaluate(() => (window as any).repairTask.done(0))
  expect(await page.evaluate(async () => (await import('/src/lib/state.svelte.ts')).app.libraryVersion)).toBe(version)
  const flow = await page.evaluate(async () => {
    const { repairFlow } = await import('/src/lib/repairflow.svelte.ts')
    return { running: repairFlow.running, written: repairFlow.result?.written, root: repairFlow.root }
  })
  expect(flow).toEqual({ running: false, written: 1, root: '/mock/library' })
})

test('repair completion checks all ownership fields and is applied only once', async ({ page }) => {
  const panel = await prepare(page)
  await panel.getByRole('button', { name: '补缺图', exact: true }).click()
  await expect.poll(() => requestCount(page)).toBe(1)
  await page.evaluate(() => (window as any).repairTask.requests[0].resolve(901))
  const version = await page.evaluate(async () => (await import('/src/lib/state.svelte.ts')).app.libraryVersion)
  await page.evaluate(() => {
    const s = (window as any).repairTask
    s.done(0, { jobId: 999 })
    s.done(0, { root: '/mock/other-library' })
    s.done(0, { code: 'OTHER-101' })
  })
  await expect(panel.getByRole('button', { name: '停止补图', exact: true })).toBeVisible()
  await page.evaluate(() => {
    const s = (window as any).repairTask
    s.done(0)
    s.done(0, { written: 99 })
  })
  await expect(panel.getByRole('status', { name: '补图结果' })).toContainText('写入 1 张图片')
  expect(await page.evaluate(async () => (await import('/src/lib/state.svelte.ts')).app.libraryVersion)).toBe(version + 1)
})

test('repair can retry after a completion listener setup failure', async ({ page }) => {
  await page.route('**/src/lib/events.ts*', async route => {
    const response = await route.fetch()
    await route.fulfill({ response, body: (await response.text()) + `
      const originalListen=listen;let failed=false;
      listen=async(event,handler)=>{
        if(event==='repair:done'&&!failed){failed=true;throw new Error('补图监听暂不可用')}
        return originalListen(event,handler);
      };
    ` })
  })
  const panel = await prepare(page)
  await panel.getByRole('button', { name: '补缺图', exact: true }).click()
  await expect(panel.getByRole('alert')).toContainText('补图监听暂不可用')
  expect(await requestCount(page)).toBe(0)
  await panel.getByRole('button', { name: '补缺图', exact: true }).click()
  await expect.poll(() => requestCount(page)).toBe(1)
  await page.evaluate(() => {
    const s = (window as any).repairTask
    s.done(0)
    s.requests[0].resolve(901)
  })
  await expect(panel.getByRole('status', { name: '补图结果' })).toContainText('写入 1 张图片')
})

test('changing library during listener setup prevents a stale repair from starting', async ({ page }) => {
  await page.route('**/src/lib/events.ts*', async route => {
    const response = await route.fetch()
    await route.fulfill({ response, body: (await response.text()) + `
      const originalListen=listen;
      listen=async(event,handler)=>{
        if(event==='repair:done')await new Promise(resolve=>window.releaseRepairListener=resolve);
        return originalListen(event,handler);
      };
    ` })
  })
  let panel = await prepare(page)
  await panel.getByRole('button', { name: '补缺图', exact: true }).click()
  await expect.poll(() => page.evaluate(() => typeof (window as any).releaseRepairListener)).toBe('function')
  await page.evaluate(async () => {
    const { app } = await import('/src/lib/state.svelte.ts')
    app.settings = { ...app.settings!, library_root: '/mock/other-library' }
    ;(window as any).repairTask.root = '/mock/other-library'
    ;(window as any).releaseRepairListener()
  })
  await expect(page.getByRole('status').filter({ hasText: '媒体库已变化，请重新补图' })).toBeVisible()
  expect(await requestCount(page)).toBe(0)
  if (await panel.count() === 0) await page.locator('button.pcard').first().click()
  panel = page.getByLabel('影片详情', { exact: true })
  await expect(panel).toHaveClass(/armed/)
  await panel.getByRole('button', { name: '补缺图', exact: true }).click()
  await expect.poll(() => requestCount(page)).toBe(1)
  await page.evaluate(() => {
    const s = (window as any).repairTask
    s.done(0)
    s.requests[0].resolve(901)
  })
  await expect(panel.getByRole('status', { name: '补图结果' })).toContainText('写入 1 张图片')
})
