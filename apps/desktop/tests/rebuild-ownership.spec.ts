import { expect, test, type Page } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

async function prepare(page: Page) {
  await seedLibrary(page)
  await overrideApi(page, `
    const s=window.rebuildTask={requests:[],notices:[],root:'/mock/library'};
    api.libraryRebuild=()=>new Promise((resolve,reject)=>s.requests.push({resolve,reject,root:s.root}));
    api.notifyFinish=async(title,body)=>{s.notices.push({title,body});return true};
    const {emitMock}=await import('/src/lib/events.ts');
    s.progress=(index,fields={})=>emitMock('library:progress',{jobId:701+index,root:s.requests[index].root,done:3,total:10,...fields});
    s.done=(index,fields={})=>emitMock('library:rebuilt',{jobId:701+index,root:s.requests[index].root,count:7,cancelled:false,error:null,...fields});
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '设置', exact: true }).click()
  await page.getByRole('button', { name: '重建索引', exact: true }).click()
  await expect.poll(() => requestCount(page)).toBe(1)
}

const requestCount = (page: Page) => page.evaluate(() => (window as any).rebuildTask.requests.length)
const notices = (page: Page) => page.evaluate(() => (window as any).rebuildTask.notices)
const version = (page: Page) => page.evaluate(async () => (await import('/src/lib/state.svelte.ts')).app.libraryVersion)

test('rebuild ownership comes from its response, never from early progress', async ({ page }) => {
  await prepare(page)
  await page.evaluate(() => {
    const s = (window as any).rebuildTask
    s.progress(0, { jobId: 700, done: 99, total: 100 })
    s.progress(0)
  })
  await expect(page.getByRole('button', { name: '重建中 0/0', exact: true })).toBeDisabled()
  await page.evaluate(() => (window as any).rebuildTask.requests[0].resolve(701))
  await expect(page.getByRole('button', { name: '重建中 3/10', exact: true })).toBeDisabled()
  await page.evaluate(() => (window as any).rebuildTask.progress(0, { jobId: 700, done: 100, total: 100 }))
  await expect(page.getByRole('button', { name: '重建中 3/10', exact: true })).toBeDisabled()
  await page.evaluate(() => (window as any).rebuildTask.done(0))
  await expect(page.getByRole('button', { name: '重建索引', exact: true })).toBeEnabled()
})

test('a foreign completion cannot report success for a rejected rebuild start', async ({ page }) => {
  await prepare(page)
  await page.evaluate(() => (window as any).rebuildTask.done(0, { jobId: 700, count: 99 }))
  await expect(page.getByRole('button', { name: '重建中 0/0', exact: true })).toBeDisabled()
  expect(await notices(page)).toEqual([])
  await expect(page.getByRole('status').filter({ hasText: '索引已重建' })).toHaveCount(0)
  await page.evaluate(() => (window as any).rebuildTask.requests[0].reject('已有任务在运行:rebuild'))
  await expect(page.getByRole('alert')).toContainText('已有任务在运行:rebuild')
  await expect(page.getByRole('button', { name: '重建索引', exact: true })).toBeEnabled()
})

test('early rebuild completion wins over progress and is applied exactly once after ownership', async ({ page }) => {
  await prepare(page)
  const before = await version(page)
  await page.evaluate(() => {
    const s = (window as any).rebuildTask
    s.done(0)
    s.done(0, { count: 99 })
    s.progress(0)
  })
  await expect(page.getByRole('button', { name: '重建中 0/0', exact: true })).toBeDisabled()
  expect(await notices(page)).toEqual([])
  await page.evaluate(() => (window as any).rebuildTask.requests[0].resolve(701))
  await expect(page.getByRole('status')).toContainText('索引已重建:7 部影片')
  await expect(page.getByRole('button', { name: '重建索引', exact: true })).toBeEnabled()
  await page.evaluate(() => (window as any).rebuildTask.done(0, { count: 99 }))
  expect(await notices(page)).toEqual([{ title: '索引重建完成', body: '共 7 部影片' }])
  expect(await version(page)).toBe(before + 1)
})

test('early rebuild errors are not hidden by unrelated completion events', async ({ page }) => {
  await prepare(page)
  await page.evaluate(() => {
    const s = (window as any).rebuildTask
    s.done(0, { jobId: 700, count: 99 })
    s.done(0, { error: '索引读取失败', count: 0 })
    s.requests[0].resolve(701)
  })
  await expect(page.getByRole('alert')).toContainText('索引读取失败')
  await expect(page.getByRole('button', { name: '重建索引', exact: true })).toBeEnabled()
  expect(await notices(page)).toEqual([{ title: '索引重建失败', body: '索引读取失败' }])
})

for (const responseFirst of [false, true]) {
  test(`old-library rebuild settles after a root change, response first: ${responseFirst}`, async ({ page }) => {
    await prepare(page)
    if (responseFirst) await page.evaluate(() => (window as any).rebuildTask.requests[0].resolve(701))
    const before = await version(page)
    await page.evaluate(async () => {
      const { app } = await import('/src/lib/state.svelte.ts')
      app.settings = { ...app.settings!, library_root: '/mock/other-library' }
      const s = (window as any).rebuildTask
      s.root = '/mock/other-library'
      s.done(0)
    })
    if (!responseFirst) await page.evaluate(() => (window as any).rebuildTask.requests[0].resolve(701))
    await expect(page.getByRole('button', { name: '重建索引', exact: true })).toBeEnabled()
    expect(await notices(page)).toEqual([])
    expect(await version(page)).toBe(before)
    await expect(page.getByRole('status').filter({ hasText: '索引已重建' })).toHaveCount(0)
    await page.getByRole('button', { name: '重建索引', exact: true }).click()
    await expect.poll(() => requestCount(page)).toBe(2)
    await page.evaluate(() => {
      const s = (window as any).rebuildTask
      s.done(0, { error: '旧库迟到错误' })
      s.requests[1].resolve(702)
      s.done(1, { count: 8 })
    })
    await expect(page.getByRole('status')).toContainText('索引已重建:8 部影片')
    await expect(page.getByRole('alert').filter({ hasText: '旧库迟到错误' })).toHaveCount(0)
    expect(await notices(page)).toEqual([{ title: '索引重建完成', body: '共 8 部影片' }])
  })
}

test('rebuild remains owned across navigation and repeated start requests', async ({ page }) => {
  await prepare(page)
  await page.evaluate(async () => {
    const { app } = await import('/src/lib/state.svelte.ts')
    await app.rebuildIndex()
    ;(window as any).rebuildTask.requests[0].resolve(701)
  })
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.evaluate(() => (window as any).rebuildTask.progress(0))
  await page.getByRole('button', { name: '设置', exact: true }).click()
  await expect(page.getByRole('button', { name: '重建中 3/10', exact: true })).toBeDisabled()
  expect(await requestCount(page)).toBe(1)
  await page.evaluate(() => (window as any).rebuildTask.done(0, { cancelled: true }))
  await expect(page.getByRole('status')).toContainText('已取消重建，保留原索引')
  expect(await notices(page)).toEqual([{ title: '索引重建已取消', body: '保留原索引' }])
})

test('failed rebuild start can retry without old events changing the new progress', async ({ page }) => {
  await prepare(page)
  await page.evaluate(() => (window as any).rebuildTask.requests[0].reject('索引暂不可用'))
  await expect(page.getByRole('alert')).toContainText('索引暂不可用')
  await page.getByRole('button', { name: '重建索引', exact: true }).click()
  await expect.poll(() => requestCount(page)).toBe(2)
  await page.evaluate(() => {
    const s = (window as any).rebuildTask
    s.progress(0, { done: 99, total: 100 })
    s.requests[1].resolve(702)
    s.progress(1)
  })
  await expect(page.getByRole('button', { name: '重建中 3/10', exact: true })).toBeDisabled()
  await page.evaluate(() => (window as any).rebuildTask.done(1))
  await expect(page.getByRole('button', { name: '重建索引', exact: true })).toBeEnabled()
})
