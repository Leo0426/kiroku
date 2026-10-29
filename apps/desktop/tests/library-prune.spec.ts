import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

test('checks on entry and focus, refreshes cards after removal without scanning every query', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    window.pruneCalls=[]; let hidden=false;
    const list=api.libraryList;
    api.libraryList=async q=>{const p=await list(q);return hidden?{...p,items:[],total:0}:p};
    api.libraryPruneMissing=async(root,automatic,requestId)=>{window.pruneCalls.push({automatic,requestId});if(!automatic)hidden=true;return {root:'/mock/library',removed:automatic?0:600,skipped:false}};
  `)
  await page.goto('/')
  await expect.poll(() => page.evaluate(() => ((window as any).pruneCalls?.length ?? 0))).toBe(1)
  await expect(page.locator('button.pcard').first()).toBeVisible()
  await page.getByRole('button', { name: '刷新媒体库', exact: true }).click()
  await expect(page.getByRole('status').filter({ hasText: '已移除 600 部已移走的影片' })).toBeVisible()
  await expect(page.locator('button.pcard')).toHaveCount(0)
  await page.evaluate(() => window.dispatchEvent(new Event('focus')))
  await expect.poll(() => page.evaluate(() => ((window as any).pruneCalls?.length ?? 0))).toBe(3)
  expect(await page.evaluate(() => (window as any).pruneCalls.map((x: any) => x.automatic))).toEqual([true, false, true])
})

test('offline errors preserve cards and retry is available', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `api.libraryPruneMissing=async()=>{throw '磁盘离线'};`)
  await page.goto('/')
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
  await expect.poll(() => page.evaluate(() => ((window as any).pruneCalls?.length ?? 0))).toBe(1)
  await page.getByRole('button', { name: '停止刷新', exact: true }).click()
  await page.evaluate(() => (window as any).started(0))
  await expect.poll(() => page.evaluate(() => (window as any).stopped)).toEqual([801])
  await page.evaluate(async () => {
    const { app } = await import('/src/lib/state.svelte.ts')
    app.settings = { ...app.settings!, library_root: '/mock/other-library' }
  })
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

test('startup cleanup waits for recovery inspection instead of being skipped by its job', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, `
    window.pruneCalls=[]; window.finishRecovery=null;
    api.recoveryInspect=()=>new Promise(resolve=>window.finishRecovery=()=>resolve({root:'/mock/library',entries:[]}));
    api.libraryPruneMissing=async root=>{window.pruneCalls.push(root);return {root,removed:0,skipped:false}};
  `)
  await page.goto('/')
  await expect(page.getByRole('button', { name: '刷新媒体库', exact: true })).toBeVisible()
  expect(await page.evaluate(() => (window as any).pruneCalls.length)).toBe(0)
  await page.evaluate(() => (window as any).finishRecovery())
  await expect.poll(() => page.evaluate(() => (window as any).pruneCalls.length)).toBe(1)
})
