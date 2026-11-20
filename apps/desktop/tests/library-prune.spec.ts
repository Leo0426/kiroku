import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

test('explicit refresh removes moved cards; entry and focus do not scan', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    window.pruneCalls=[]; let hidden=false;
    const list=api.libraryList;
    api.libraryList=async q=>{const p=await list(q);return hidden?{...p,items:[],total:0}:p};
    api.libraryPruneMissing=async(root,automatic,requestId)=>{window.pruneCalls.push({automatic,requestId});if(!automatic)hidden=true;return {root:'/mock/library',removed:automatic?0:600,skipped:false}};
  `)
  await page.goto('/')
  await expect(page.locator('button.pcard').first()).toBeVisible()
  expect(await page.evaluate(() => (window as any).pruneCalls)).toEqual([])
  await page.getByRole('button', { name: '刷新媒体库', exact: true }).click()
  await expect(page.getByRole('status').filter({ hasText: '已移除 600 部已移走的影片' })).toBeVisible()
  await expect(page.locator('button.pcard')).toHaveCount(0)
  await page.evaluate(() => window.dispatchEvent(new Event('focus')))
  await expect(page.getByRole('button', { name: '刷新媒体库', exact: true })).toBeEnabled()
  expect(await page.evaluate(() => (window as any).pruneCalls.map((x: any) => x.automatic))).toEqual([false])
})

test('offline errors preserve cards and retry is available', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `api.libraryPruneMissing=async()=>{throw '磁盘离线'};`)
  await page.goto('/')
  await page.getByRole('button', { name: '刷新媒体库', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('磁盘离线')
  await expect(page.locator('button.pcard').first()).toBeVisible()
  await expect(page.getByRole('button', { name: '刷新媒体库', exact: true })).toBeEnabled()
})

test('early stop targets only the owned job and late completion cannot leak across library changes', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    window.pruneCalls=[];window.stopped=[];
    const {emitMock}=await import('/src/lib/events.ts');
    api.libraryPruneMissing=(root,automatic,requestId)=>new Promise(resolve=>window.pruneCalls.push({requestId,resolve}));
    window.started=i=>emitMock('job:started',{jobId:801+i,requestId:window.pruneCalls[i].requestId});
    api.jobCancel=async id=>{window.stopped.push(id);return true};
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '刷新媒体库', exact: true }).click()
  await expect.poll(() => page.evaluate(() => ((window as any).pruneCalls?.length ?? 0))).toBe(1)
  await page.getByRole('button', { name: '停止刷新', exact: true }).click()
  await page.evaluate(() => (window as any).started(0))
  await expect.poll(() => page.evaluate(() => (window as any).stopped)).toEqual([801])
  await page.evaluate(async () => {
    const { app } = await import('/src/lib/state.svelte.ts')
    app.settings = { ...app.settings!, library_root: '/mock/other-library' }
  })
  await page.getByRole('button', { name: '刷新媒体库', exact: true }).click()
  await expect.poll(() => page.evaluate(() => ((window as any).pruneCalls?.length ?? 0))).toBe(2)
  await page.evaluate(() => (window as any).pruneCalls[0].resolve({ root: '/mock/library', removed: 42, skipped: false }))
  await expect(page.getByRole('button', { name: '正在刷新…', exact: true })).toBeDisabled()
  await expect(page.getByText('已移除 42 部已移走的影片')).toHaveCount(0)
  await page.evaluate(() => (window as any).pruneCalls[1].resolve({ root: '/mock/other-library', removed: 0, skipped: false }))
  await expect(page.getByRole('button', { name: '刷新媒体库', exact: true })).toBeEnabled()
})

test('a change event from an old library cannot refresh the new library', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, 'api.libraryPruneMissing=async root=>({root,removed:0,skipped:false});')
  await page.goto('/')
  await expect(page.locator('button.pcard').first()).toBeVisible()
  const result = await page.evaluate(async () => {
    const { app } = await import('/src/lib/state.svelte.ts')
    const { emitMock } = await import('/src/lib/events.ts')
    const before = app.libraryVersion
    emitMock('library:changed', { jobId: 901, root: '/mock/old-library' } as any)
    const afterOld = app.libraryVersion
    emitMock('library:changed', { jobId: 902, root: '/mock/library' } as any)
    return { before, afterOld, afterCurrent: app.libraryVersion }
  })
  expect(result.afterOld).toBe(result.before)
  expect(result.afterCurrent).toBe(result.before + 1)
})

test('explicit refresh works during recovery inspection and is never started automatically', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    window.pruneCalls=[]; window.finishRecovery=null;
    api.recoveryInspect=()=>new Promise(resolve=>window.finishRecovery=()=>resolve({root:'/mock/library',entries:[]}));
    api.libraryPruneMissing=async root=>{window.pruneCalls.push(root);return {root,removed:0,skipped:false}};
  `)
  await page.goto('/')
  await expect(page.getByRole('button', { name: '刷新媒体库', exact: true })).toBeVisible()
  expect(await page.evaluate(() => (window as any).pruneCalls.length)).toBe(0)
  await expect(page.getByText('正在检查整理日志…', {exact:true})).toBeVisible()
  await page.getByRole('button', { name: '刷新媒体库', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (window as any).pruneCalls.length)).toBe(1)
  await page.evaluate(() => (window as any).finishRecovery())
  await expect(page.getByText('正在检查整理日志…', {exact:true})).toBeHidden()
})

test('entry and window focus do not reserve the job needed to open a crop', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    window.pruneCalls=[]; let pruning=false;
    api.libraryPruneMissing=(root,automatic)=>{
      window.pruneCalls.push({automatic}); pruning=true;
      return new Promise(()=>{});
    };
    const open=api.artworkOpen;
    api.artworkOpen=async(...args)=>{
      if(pruning) throw new Error('已有任务在运行:library-prune');
      return open(...args);
    };
  `)
  await page.goto('/')
  await page.locator('button.pcard').first().click()
  await page.evaluate(() => window.dispatchEvent(new Event('focus')))
  await page.getByRole('button', { name: '裁剪海报', exact: true }).click()
  await expect(page.getByAltText('裁剪原图')).toBeVisible()
  expect(await page.evaluate(() => (window as any).pruneCalls)).toEqual([])
})

test('refresh keeps missing records until the user confirms their exact codes', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    window.requests=[];
    api.libraryPruneMissing=async(root,automatic,id,codes)=>{
      window.requests.push(codes);
      return {root,skipped:false,removed:codes?.length??0,relocated:1,refreshed:1,ambiguous:1,
        missingCodes:codes?.length ? [] : ['TEST-001','TEST-002']};
    };
  `)
  await page.goto('/')
  const refresh = page.getByRole('button', { name: '刷新媒体库', exact:true })
  await refresh.click()
  const review = page.getByRole('alertdialog', {name:'有 2 部影片未找到'})
  await expect(review).toContainText('保留磁盘上的文件')
  await expect(review.getByRole('button', {name:'保留记录'})).toBeFocused()
  await review.getByText('查看影片编号').click()
  await expect(review).toContainText('TEST-001')
  await review.getByRole('button', {name:'保留记录'}).click()
  expect(await page.evaluate(() => (window as any).requests)).toEqual([[]])
  await refresh.click()
  await review.getByRole('button', {name:'移除这 2 条索引'}).click()
  await expect.poll(() => page.evaluate(() => (window as any).requests)).toEqual([[],[],['TEST-001','TEST-002']])
  await expect(review).toBeHidden()
})

test('a failed poster can be retried alone and exactly once after an explicit refresh', async ({ page }) => {
  await seedLibrary(page)
  const reads = [0,0,0]
  let restored = false
  await page.route('**/fixture-posters/**', async route => {
    const index = Number(new URL(route.request().url()).pathname.split('/').pop())
    reads[index]++
    if (index === 0 || (index === 1 && restored)) {
      await route.fulfill({ contentType:'image/svg+xml', body:'<svg xmlns="http://www.w3.org/2000/svg" width="4" height="6"/>' })
    } else await route.fulfill({status:404, body:''})
  })
  await overrideApi(page, `
    const list=api.libraryList;
    api.libraryList=async q=>{const p=await list(q);return {...p,total:3,items:p.items.slice(0,3).map((m,i)=>({...m,code:'TEST-00'+i,poster:location.origin+'/fixture-posters/'+i,posterVersion:'unchanged'}))}};
    window.scans=0;
    api.libraryPruneMissing=async root=>{window.scans++;return {root,removed:0,skipped:false}};
  `)
  await page.goto('/')
  const retry = (code:string) => page.getByRole('button', {name:'重试 '+code+' 的海报',exact:true})
  await expect(retry('TEST-001')).toBeVisible()
  await expect(retry('TEST-002')).toBeVisible()
  expect(reads).toEqual([1,1,1])
  const cards = page.locator('button.pcard')
  await cards.nth(0).focus()
  await page.keyboard.press('ArrowRight')
  await expect(cards.nth(1)).toBeFocused()
  await page.keyboard.press('Tab')
  await expect(retry('TEST-001')).toBeFocused()
  await page.keyboard.press('ArrowRight')
  await expect(cards.nth(2)).toBeFocused()
  await page.keyboard.press('ArrowLeft')
  await expect(cards.nth(1)).toBeFocused()
  await page.keyboard.press('Tab')
  await page.keyboard.press('Enter')
  const notice = page.getByRole('status', {name:'仍无法读取 TEST-001 的海报'})
  await expect(notice).toContainText('媒体盘')
  expect(reads).toEqual([1,2,1])
  expect(await page.evaluate(() => (window as any).scans)).toBe(0)
  restored = true
  await notice.getByRole('button', {name:'刷新媒体库'}).click()
  await expect(retry('TEST-001')).toBeHidden()
  await expect.poll(() => reads.slice()).toEqual([1,3,2])
  await page.evaluate(async () => {
    const { app } = await import('/src/lib/state.svelte.ts')
    app.libraryVersion++
  })
  await expect(page.locator('.scroller')).toHaveAttribute('aria-busy','false')
  await expect(retry('TEST-002')).toBeVisible()
  expect(reads).toEqual([1,3,2])
  expect(await page.evaluate(() => (window as any).scans)).toBe(1)
})
