import { expect, test, type Page } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

type Kind = 'sources' | 'health' | 'preview' | 'collect'
const stopLabel = { sources: '停止检测', health: '停止检查', preview: '停止维护任务', collect: '停止维护任务' }

async function prepare(page: Page, kind: Kind) {
  await seedLibrary(page)
  await overrideApi(page, `
    const kind = ${JSON.stringify(kind)};
    const preview = {token:'owned',root:'/mock/library',logDir:'/mock/logs',policy:{historyDays:90,logDays:90},scannedAt:1,
      candidateBytes:12,protected:[],candidates:[{id:'original:unused',kind:'original',paths:['/mock/library/original.png'],bytes:12,description:'旧原图'}]};
    const value = kind==='sources' ? [] : kind==='health' ? {root:'/mock/library',scannedAt:1,scanned:0,cancelled:true,issues:[]}
      : kind==='preview' ? preview : {removed:0,skipped:[],failed:[],cancelled:true};
    const state = window.utility = {cancels:[],requestId:null,failCancel:false};
    const pending = requestId => {state.requestId=requestId;return new Promise((resolve,reject)=>{state.finish=()=>resolve(value);state.reject=reject})};
    api.jobCancel=async id=>{state.cancels.push(id??null);if(state.failCancel)throw '停止失败';return true};
    if(kind==='sources')api.sourcesCheck=(_ids,_draft,requestId)=>pending(requestId);
    if(kind==='health')api.libraryHealth=requestId=>pending(requestId);
    if(kind==='preview')api.maintenancePreview=(_recent,requestId)=>pending(requestId);
    if(kind==='collect'){api.maintenancePreview=async()=>preview;api.maintenanceApply=(_token,_ids,requestId)=>pending(requestId)};
    const {emitMock}=await import('/src/lib/events.ts');
    state.started=(id,requestId=state.requestId)=>emitMock('job:started',{jobId:id,requestId});
  `)
  await page.goto('/')
  if (kind === 'sources') {
    await page.getByRole('button', { name: '设置', exact: true }).click()
    await page.getByRole('tab', { name: '数据源', exact: true }).click()
    await page.getByRole('button', { name: '检测已用来源', exact: true }).click()
  } else if (kind === 'health') {
    await page.getByRole('button', { name: '媒体库工具', exact: true }).click()
    await page.getByRole('button', { name: '检查媒体库', exact: true }).click()
  } else {
    await page.getByRole('button', { name: '历史', exact: true }).click()
    await page.getByRole('button', { name: '空间维护', exact: true }).click()
    await page.getByRole('button', { name: '预览空间清理', exact: true }).click()
    if (kind === 'collect') {
      await page.getByRole('checkbox', { name: '选择 original:unused', exact: true }).check()
      await page.getByRole('button', { name: '清理选中项', exact: true }).click()
      await page.getByRole('alertdialog').getByRole('button', { name: '移到废纸篓', exact: true }).click()
    }
  }
  await expect.poll(() => page.evaluate(() => typeof (window as any).utility.finish)).toBe('function')
}

for (const kind of ['sources', 'health', 'preview', 'collect'] as const) {
  test(`${kind}: early stop waits for its request and ignores other starts`, async ({ page }) => {
    await prepare(page, kind)
    await page.getByRole('button', { name: stopLabel[kind], exact: true }).click()
    expect(await page.evaluate(() => (window as any).utility.cancels)).toEqual([])
    expect(await page.evaluate(() => typeof (window as any).utility.requestId)).toBe('string')
    await page.evaluate(() => (window as any).utility.started(801, 'other-request'))
    expect(await page.evaluate(() => (window as any).utility.cancels)).toEqual([])
    await page.evaluate(() => (window as any).utility.started(802))
    await expect.poll(() => page.evaluate(() => (window as any).utility.cancels)).toEqual([802])
    await page.evaluate(() => { (window as any).utility.started(803); (window as any).utility.finish() })
    await expect(page.getByRole('button', { name: stopLabel[kind], exact: true })).toHaveCount(0)
    await page.evaluate(() => (window as any).utility.started(804))
    expect(await page.evaluate(() => (window as any).utility.cancels)).toEqual([802])
  })

  test(`${kind}: a rejected start cannot cancel the task occupying the slot`, async ({ page }) => {
    await prepare(page, kind)
    await page.getByRole('button', { name: stopLabel[kind], exact: true }).click()
    await page.evaluate(() => (window as any).utility.reject('已有任务在运行:repair'))
    await expect(page.getByRole('alert')).toContainText('已有任务在运行:repair')
    expect(await page.evaluate(() => (window as any).utility.cancels)).toEqual([])
  })

  test(`${kind}: leaving the page preserves an early stop for its own task`, async ({ page }) => {
    await prepare(page, kind)
    await page.getByRole('button', { name: '导入整理', exact: true }).click()
    await page.evaluate(() => (window as any).utility.started(805))
    await expect.poll(() => page.evaluate(() => (window as any).utility.cancels)).toEqual([805])
    await page.evaluate(() => (window as any).utility.finish())
    await expect(page.getByRole('button', { name: '添加文件夹', exact: true })).toBeVisible()
  })

  test(`${kind}: failed cancellation is shown without releasing the pending task`, async ({ page }) => {
    await prepare(page, kind)
    await page.evaluate(() => { const s=(window as any).utility;s.started(806);s.failCancel=true })
    await page.getByRole('button', { name: stopLabel[kind], exact: true }).click()
    await expect(page.getByRole('alert')).toContainText('停止失败')
    await expect(page.getByRole('button', { name: stopLabel[kind], exact: true })).toBeVisible()
    await page.evaluate(() => (window as any).utility.failCancel=false)
    await page.getByRole('button', { name: stopLabel[kind], exact: true }).click()
    expect(await page.evaluate(() => (window as any).utility.cancels)).toEqual([806,806])
    await page.evaluate(() => (window as any).utility.finish())
  })

  test(`${kind}: changing configuration retires the request and ignores its late error`, async ({ page }) => {
    await prepare(page, kind)
    await page.evaluate(async () => {
      const {app}=await import('/src/lib/state.svelte.ts')
      app.settings={...app.settings!,library_root:'/mock/other-library'}
    })
    await expect(page.getByRole('button', { name: stopLabel[kind], exact: true })).toHaveCount(0)
    await page.evaluate(() => (window as any).utility.started(807))
    await expect.poll(() => page.evaluate(() => (window as any).utility.cancels)).toEqual([807])
    await page.evaluate(() => (window as any).utility.reject('旧请求失败'))
    await expect(page.getByRole('alert').filter({hasText:'旧请求失败'})).toHaveCount(0)
  })
}

test('saving retention settings never offers a global stop action', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, 'api.saveSettings=async()=>new Promise(resolve=>window.finishSettings=resolve);')
  await page.goto('/')
  await page.getByRole('button', { name: '历史', exact: true }).click()
  await page.getByRole('button', { name: '空间维护', exact: true }).click()
  await page.getByRole('button', { name: '保存保留期', exact: true }).click()
  await expect.poll(() => page.evaluate(() => typeof (window as any).finishSettings)).toBe('function')
  await expect(page.getByRole('button', { name: '停止维护任务', exact: true })).toHaveCount(0)
  await page.evaluate(() => (window as any).finishSettings())
})

test('mock utility commands announce ownership and honour targeted cancellation', async ({ page }) => {
  await seedLibrary(page); await page.goto('/')
  const result = await page.evaluate(async () => {
    const {api}=await import('/src/lib/api.ts')
    const {listen}=await import('/src/lib/events.ts')
    const requests:string[]=[]
    const stop=await listen('job:started', event=>{requests.push(event.requestId);void api.jobCancel(event.jobId)})
    const health=await api.libraryHealth('health-test')
    const sources=await api.sourcesCheck(['javdb'],undefined,'sources-test')
    let previewError=''
    try{await api.maintenancePreview(false,'preview-test')}catch(e){previewError=String(e)}
    stop()
    const preview=await api.maintenancePreview()
    const stopApply=await listen('job:started', event=>{requests.push(event.requestId);void api.jobCancel(event.jobId)})
    const collected=await api.maintenanceApply(preview.token,preview.candidates.map(c=>c.id),'collect-test')
    stopApply()
    const remaining=await api.maintenancePreview()
    return {requests,healthCancelled:health.cancelled,sourceReason:sources[0].outcome.reason,previewError,collected,remaining:remaining.candidates.length}
  })
  expect(result.requests).toEqual(['health-test','sources-test','preview-test','collect-test'])
  expect(result.healthCancelled).toBe(true)
  expect(result.sourceReason).toBe('cancelled')
  expect(result.previewError).toContain('取消')
  expect(result.collected).toMatchObject({removed:0,cancelled:true})
  expect(result.remaining).toBe(1)
})
