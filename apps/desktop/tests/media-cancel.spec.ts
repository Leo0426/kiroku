import { expect, test, type Page } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

type Kind = 'play' | 'reveal'
const button = (kind: Kind) => kind === 'play' ? '播放' : '在文件夹中显示'
async function prepare(page: Page, kind: Kind) {
  await seedLibrary(page)
  await overrideApi(page, `
    const kind=${JSON.stringify(kind)};
    const state=window.mediaTask={requests:[],cancels:[],root:'/mock/library',failCancel:false};
    api[kind]=(code,part,requestId)=>new Promise((resolve,reject)=>state.requests.push({code,part,requestId,root:state.root,finish:resolve,reject}));
    api.jobCancel=async id=>{state.cancels.push(id??null);if(state.failCancel)throw '停止失败';return true};
    const {emitMock}=await import('/src/lib/events.ts');
    state.started=(index,id,requestId=state.requests[index].requestId)=>emitMock('job:started',{jobId:id,requestId});
    state.progress=(index,id,done=25)=>{const r=state.requests[index];emitMock('media:progress',{jobId:id,root:r.root,code:r.code,kind,doneBytes:done,totalBytes:100})};
  `)
  await page.goto('/')
  await page.locator('button.pcard').first().click()
  const panel=page.getByLabel('影片详情', { exact: true })
  await expect(panel).toHaveClass(/armed/)
  await panel.getByRole('button', { name: button(kind), exact: true }).first().click()
  await expect.poll(() => page.evaluate(() => (window as any).mediaTask.requests.length)).toBe(1)
  return panel
}

for (const kind of ['play', 'reveal'] as const) {
  test(`${kind}: early stop waits for its own request and ignores foreign progress`, async ({ page }) => {
    const panel=await prepare(page,kind)
    const stop=panel.getByRole('button', { name: '取消媒体校验', exact: true })
    await expect(stop).toBeEnabled()
    await stop.click()
    expect(await page.evaluate(() => (window as any).mediaTask.cancels)).toEqual([])
    expect(await page.evaluate(() => typeof (window as any).mediaTask.requests[0].requestId)).toBe('string')
    await page.evaluate(() => { const s=(window as any).mediaTask;s.started(0,801,'foreign');s.progress(0,801,99) })
    expect(await page.evaluate(() => (window as any).mediaTask.cancels)).toEqual([])
    await expect(panel.getByRole('status', { name: '媒体校验进度', exact: true })).not.toContainText('99%')
    await page.evaluate(() => { const s=(window as any).mediaTask;s.started(0,802);s.progress(0,802);s.progress(0,803,99) })
    await expect.poll(() => page.evaluate(() => (window as any).mediaTask.cancels)).toEqual([802])
    await expect(panel.getByRole('status', { name: '媒体校验进度', exact: true })).toContainText('25%')
    await page.evaluate(() => (window as any).mediaTask.requests[0].finish())
    await expect(stop).toHaveCount(0)
    await expect(page.getByRole('status').filter({ hasText: '已交给系统播放器打开' })).toHaveCount(0)
    await page.evaluate(() => (window as any).mediaTask.started(0,804))
    expect(await page.evaluate(() => (window as any).mediaTask.cancels)).toEqual([802])
  })

  test(`${kind}: a busy rejection cannot cancel another matching media job`, async ({ page }) => {
    const panel=await prepare(page,kind)
    await page.evaluate(() => (window as any).mediaTask.progress(0,805))
    await panel.getByRole('button', { name: '取消媒体校验', exact: true }).click()
    await page.evaluate(() => (window as any).mediaTask.requests[0].reject('已有任务在运行:play'))
    await expect(page.getByRole('alert')).toContainText('已有任务在运行:play')
    expect(await page.evaluate(() => (window as any).mediaTask.cancels)).toEqual([])
  })

  test(`${kind}: closing details keeps cancellation pending until ownership arrives`, async ({ page }) => {
    const panel=await prepare(page,kind)
    await panel.getByRole('button', { name: '关闭详情', exact: true }).click()
    await expect(panel).toHaveCount(0)
    await page.evaluate(() => (window as any).mediaTask.started(0,806))
    await expect.poll(() => page.evaluate(() => (window as any).mediaTask.cancels)).toEqual([806])
    await page.evaluate(() => (window as any).mediaTask.requests[0].reject('旧校验失败'))
    await expect(page.getByRole('alert').filter({ hasText: '旧校验失败' })).toHaveCount(0)
  })

  test(`${kind}: failed cancellation remains retryable`, async ({ page }) => {
    const panel=await prepare(page,kind)
    await page.evaluate(() => { const s=(window as any).mediaTask;s.started(0,807);s.progress(0,807);s.failCancel=true })
    await panel.getByRole('button', { name: '取消媒体校验', exact: true }).click()
    await expect(page.getByRole('alert')).toContainText('停止失败')
    await expect(panel.getByRole('button', { name: button(kind), exact: true }).first()).toBeDisabled()
    await page.evaluate(() => (window as any).mediaTask.failCancel=false)
    await panel.getByRole('button', { name: '取消媒体校验', exact: true }).click()
    expect(await page.evaluate(() => (window as any).mediaTask.cancels)).toEqual([807,807])
    await page.evaluate(() => (window as any).mediaTask.requests[0].finish())
    await expect(panel.getByRole('button', { name: button(kind), exact: true }).first()).toBeEnabled()
  })

  test(`${kind}: changing library retires the old validation without disabling the next one`, async ({ page }) => {
    let panel=await prepare(page,kind)
    await page.evaluate(async () => {
      const {app}=await import('/src/lib/state.svelte.ts')
      ;(window as any).mediaTask.root='/mock/other-library'
      app.settings={...app.settings!,library_root:'/mock/other-library'}
    })
    await expect(page.getByRole('button', { name: '取消媒体校验', exact: true })).toHaveCount(0)
    await page.evaluate(() => (window as any).mediaTask.started(0,808))
    await expect.poll(() => page.evaluate(() => (window as any).mediaTask.cancels)).toEqual([808])
    // Reopen if the library view closed the panel during its root change.
    if (await panel.count() === 0) await page.locator('button.pcard').first().click()
    panel=page.getByLabel('影片详情', { exact: true })
    await expect(panel).toHaveClass(/armed/)
    await panel.getByRole('button', { name: button(kind), exact: true }).first().click()
    await expect.poll(() => page.evaluate(() => (window as any).mediaTask.requests.length)).toBe(2)
    await page.evaluate(() => { const s=(window as any).mediaTask;s.started(1,809);s.requests[0].reject('旧校验失败') })
    await page.evaluate(() => (window as any).mediaTask.progress(1,809,50))
    await expect(panel.getByRole('status', { name: '媒体校验进度', exact: true })).toContainText('50%')
    await expect(page.getByRole('alert').filter({ hasText: '旧校验失败' })).toHaveCount(0)
    await panel.getByRole('button', { name: '取消媒体校验', exact: true }).click()
    expect(await page.evaluate(() => (window as any).mediaTask.cancels)).toEqual([808,809])
    await page.evaluate(() => (window as any).mediaTask.requests[1].finish())
  })
}

test('mock media commands announce ownership and cancellation prevents opening', async ({ page }) => {
  await seedLibrary(page)
  const opened:string[]=[]
  page.on('console', message => { if (/^\[mock\] (play|reveal) /.test(message.text())) opened.push(message.text()) })
  await page.goto('/')
  await expect(page.locator('button.pcard').first()).toBeVisible()
  const result=await page.evaluate(async () => {
    const {api}=await import('/src/lib/api.ts')
    const {listen}=await import('/src/lib/events.ts')
    const code=(await api.libraryList({limit:1})).items[0].code
    const requests:string[]=[],errors:string[]=[]
    const stop=await listen('job:started', event=>{requests.push(event.requestId);void api.jobCancel(event.jobId)})
    for (const kind of ['play','reveal'] as const) {
      try { await api[kind](code,undefined,`cancel-${kind}`);errors.push('unexpected success') }
      catch (error) { errors.push(String(error)) }
    }
    stop()
    return {requests,errors}
  })
  expect(result.requests).toEqual(['cancel-play','cancel-reveal'])
  expect(result.errors).toHaveLength(2)
  for(const error of result.errors)expect(error).toContain('取消')
  expect(opened).toEqual([])
})

test.describe('animated detail close', () => {
  test.use({ reducedMotion: 'no-preference' })
  for (const kind of ['play','reveal'] as const) {
   for (const close of ['button','escape','backdrop'] as const) {
    test(`${kind}: ${close} records stop before a response arriving during the outro`, async ({ page }) => {
      const panel=await prepare(page,kind)
      if(close==='button')await panel.getByRole('button', { name: '关闭详情', exact: true }).click()
      else if(close==='escape')await page.keyboard.press('Escape')
      else await page.locator('.scrim').click({position:{x:300,y:150}})
      // Do not wait for the closing animation: the backend can finish now.
      await page.evaluate(() => { const s=(window as any).mediaTask;s.started(0,810);s.requests[0].finish() })
      await expect(panel).toHaveCount(0)
      expect(await page.evaluate(() => (window as any).mediaTask.cancels)).toEqual([810])
      await expect(page.getByRole('status').filter({ hasText: '已交给系统播放器打开' })).toHaveCount(0)
    })
   }
  }
})

for(const kind of ['play','reveal'] as const) {
 for(const retire of ['close','library'] as const) {
  test(`${kind}: ${retire} retains a scoped retry when cancellation fails`, async ({page})=>{
    const panel=await prepare(page,kind)
    await page.clock.install()
    await page.evaluate(()=>(window as any).mediaTask.failCancel=true)
    if(retire==='close')await panel.getByRole('button',{name:'关闭详情',exact:true}).click()
    else await page.evaluate(async()=>{
      const {app}=await import('/src/lib/state.svelte.ts')
      app.settings={...app.settings!,library_root:'/mock/other-library'}
    })
    await page.evaluate(()=>(window as any).mediaTask.started(0,811))
    const alert=page.getByRole('alert').filter({hasText:'停止失败'})
    await expect(alert).toBeVisible()
    await page.clock.fastForward(6000)
    await expect(alert).toBeVisible()
    // A different task announcement must never change this retry's target.
    await page.evaluate(()=>(window as any).mediaTask.started(0,812,'new-request'))
    await alert.getByRole('button',{name:'重试停止',exact:true}).click()
    await expect.poll(()=>page.evaluate(()=>(window as any).mediaTask.cancels)).toEqual([811,811])
    await expect(alert).toHaveCount(1)
    if(retire==='library') {
      await page.evaluate(()=>(window as any).mediaTask.failCancel=false)
      await alert.getByRole('button',{name:'重试停止',exact:true}).click()
      await expect.poll(()=>page.evaluate(()=>(window as any).mediaTask.cancels)).toEqual([811,811,811])
    }
    await page.evaluate(()=>(window as any).mediaTask.requests[0].finish())
    await expect(alert).toHaveCount(0)
    await expect(page.getByRole('button',{name:'重试停止',exact:true})).toHaveCount(0)
  })
 }
}
