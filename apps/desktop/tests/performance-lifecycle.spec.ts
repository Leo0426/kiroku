import { expect, test, type Page } from '@playwright/test'
import { seedLibrary, overrideApi } from './helpers'

test.beforeEach(async ({ page }) => seedLibrary(page))
const nav = (page: Page, name: string) => page.getByRole('navigation', { name: '主导航' }).getByRole('button', { name, exact: true })
async function previewBatch(page: Page) {
  await page.getByRole('button', { name: '选择影片批量编辑' }).click()
  await page.getByLabel(/^批量选择 /).first().check()
  await page.getByRole('button', { name: '预览批量修改' }).click()
}

test('typing dispatches only the debounced list and overview query', async ({ page }) => {
  await overrideApi(page, `
    const list=api.libraryList,overview=api.libraryOverview;
    window.listQueries=[];window.overviewQueries=[];
    api.libraryList=q=>{window.listQueries.push(q.text);return list(q)};
    api.libraryOverview=q=>{window.overviewQueries.push(q.text);return overview(q)};
  `)
  await page.goto('/')
  await expect(page.locator('button.pcard').first()).toBeVisible()
  await page.getByRole('button', { name: '媒体库工具', exact: true }).click()
  await expect(page.getByRole('region', { name: '当前筛选概览' }).locator('dd').first()).toBeVisible()
  await page.evaluate(() => { (window as any).listQueries=[]; (window as any).overviewQueries=[] })
  await page.getByLabel('搜索', { exact: true }).pressSequentially('abcdefghijklm', { delay: 40 })
  await expect.poll(() => page.evaluate(() => (window as any).overviewQueries)).toEqual(['abcdefghijklm'])
  await expect.poll(() => page.evaluate(() => (window as any).listQueries)).toEqual(['abcdefghijklm'])
})

test('cancelled overview waiters settle and release heap while the transport stays occupied', async ({ page }) => {
  await page.goto('/')
  await expect(page.locator('button.pcard').first()).toBeVisible()
  await page.evaluate(async () => {
    const { api } = await import('/src/lib/api.ts' as string)
    const { overviewRequest } = await import('/src/lib/overview-requests.ts' as string)
    const w = window as any
    w.overviewCalls=0;w.settled=0
    api.libraryOverview=()=>{w.overviewCalls++;return new Promise(resolve=>w.releaseOverview=resolve)}
    void overviewRequest({},new AbortController().signal)
    w.enqueue=()=>{
      for(let i=0;i<1000;i++) {
        const controller=new AbortController()
        const text=JSON.parse(JSON.stringify(`${i}:`+'x'.repeat(16384)))
        overviewRequest({text},controller.signal).then(()=>w.settled++,()=>w.settled++)
        controller.abort()
      }
    }
  })
  const cdp=await page.context().newCDPSession(page)
  const heap=async()=>{await cdp.send('HeapProfiler.collectGarbage');return (await cdp.send('Runtime.getHeapUsage')).usedSize}
  const before=await heap()
  await page.evaluate(()=>(window as any).enqueue())
  await expect.poll(()=>page.evaluate(()=>(window as any).settled)).toBe(1000)
  expect(await page.evaluate(()=>(window as any).overviewCalls)).toBe(1)
  const after=await heap()
  console.log(JSON.stringify({cancelledOverviewWaiters:1000,before,after,growth:after-before}))
  expect(after-before).toBeLessThan(2*1024*1024)
  await page.evaluate(()=>(window as any).releaseOverview({movies:0}))
  const recovered=await page.evaluate(async()=>{
    const {api}=await import('/src/lib/api.ts' as string)
    const {overviewRequest}=await import('/src/lib/overview-requests.ts' as string)
    api.libraryOverview=()=>{throw new Error('synthetic failure')}
    await overviewRequest({},new AbortController().signal).catch(()=>{})
    api.libraryOverview=async()=>({movies:3})
    return overviewRequest({},new AbortController().signal)
  })
  expect(recovered.movies).toBe(3)
})

for (const exit of ['return','close','leave'] as const) {
  test(`abandoned batch preview releases the backend token on ${exit}`, async ({page})=>{
    await overrideApi(page,`const preview=api.batchEditPreview,discard=api.batchEditDiscard;window.discarded=[];api.batchEditPreview=async(...args)=>{const p=await preview(...args);window.batchPreview=p;return p};api.batchEditDiscard=async token=>{window.discarded.push(token);return discard(token)};`)
    await page.goto('/')
    await page.getByRole('button',{name:'媒体库工具',exact:true}).click()
    await previewBatch(page)
    await expect(page.getByLabel('批量编辑差异')).toBeVisible()
    if(exit==='return')await page.getByRole('button',{name:'返回修改选择'}).click()
    if(exit==='close')await page.getByRole('button',{name:'关闭批量编辑'}).click()
    if(exit==='leave')await nav(page,'历史').click()
    await expect.poll(()=>page.evaluate(()=>(window as any).discarded.length)).toBe(1)
    const result=await page.evaluate(async()=>{
      const {api}=await import('/src/lib/api.ts' as string);const p=(window as any).batchPreview
      let error='';try{await api.batchEditApply(p.token,p.root)}catch(e){error=String(e)}
      return {error,fields:Object.keys(p.items[0]).sort()}
    })
    expect(result.error).toContain('预览已过期')
    expect(result.fields).toEqual(['changes','code','warnings'])
  })
}

test('a preview arriving after unmount is discarded without releasing newer tokens',async({page})=>{
  await overrideApi(page,`const preview=api.batchEditPreview,discard=api.batchEditDiscard;window.discarded=[];window.holdBatch=true;api.batchEditPreview=async(...args)=>{const p=await preview(...args);window.batchPreview=p;if(window.holdBatch)await new Promise(resolve=>window.releaseBatch=resolve);return p};api.batchEditDiscard=async token=>{window.discarded.push(token);return discard(token)};`)
  await page.goto('/');await page.getByRole('button',{name:'媒体库工具',exact:true}).click()
  await previewBatch(page)
  await expect.poll(()=>page.evaluate(()=>!!(window as any).releaseBatch)).toBe(true)
  await nav(page,'历史').click()
  await page.evaluate(()=>(window as any).releaseBatch())
  await expect.poll(()=>page.evaluate(()=>(window as any).discarded.length)).toBe(1)
  await page.evaluate(()=>(window as any).holdBatch=false)
  const result=await page.evaluate(async()=>{
    const {api}=await import('/src/lib/api.ts' as string)
    const code=(await api.libraryList({limit:1})).items[0].code
    const preview=()=>api.batchEditPreview([code],{kind:'favorite',favorite:true})
    const old=await preview(),current=await preview()
    await api.batchEditDiscard(old.token)
    try{await api.batchEditApply(old.token,old.root)}catch{}
    return api.batchEditApply(current.token,current.root)
  })
  expect(result.results).toHaveLength(1)
})

test('batch apply failure before token consumption releases its preview',async({page})=>{
  await overrideApi(page,`const discard=api.batchEditDiscard;window.discarded=[];api.batchEditApply=async()=>{throw '任务已被占用'};api.batchEditDiscard=async token=>{window.discarded.push(token);return discard(token)};`)
  await page.goto('/');await page.getByRole('button',{name:'媒体库工具',exact:true}).click()
  await previewBatch(page);await expect(page.getByLabel('批量编辑差异')).toBeVisible()
  await page.getByRole('button',{name:'确认保存批量修改'}).click()
  await expect(page.getByRole('region',{name:'批量收藏与标签'})).toContainText('任务已被占用')
  await expect.poll(()=>page.evaluate(()=>(window as any).discarded.length)).toBe(1)
})
