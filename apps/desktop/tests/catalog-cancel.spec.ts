import { expect, test, type Page } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

type Phase = 'inventory' | 'preview' | 'apply'
async function prepare(page: Page, phase: Phase) {
  await seedLibrary(page)
  await overrideApi(page, `
    const phase=${JSON.stringify(phase)};
    const state=window.catalogTask={cancels:[],requestId:null,failCancel:false};
    const value=phase==='inventory'?{movies:[],names:{},errors:[],cancelled:true}
      :phase==='preview'?{token:'late-preview',root:'/mock/library',fingerprint:'rules',items:[],warnings:[]}
      :{jobId:902,items:[],cancelled:true,indexError:null};
    state.value=value;
    const pending=requestId=>{state.requestId=requestId;return new Promise((resolve,reject)=>{state.finish=()=>resolve(state.value);state.reject=reject})};
    if(phase==='inventory')api.catalogInventory=requestId=>pending(requestId);
    if(phase==='preview')api.catalogPreview=(_dirs,requestId)=>pending(requestId);
    if(phase==='apply')api.catalogApply=(_token,requestId)=>pending(requestId);
    api.jobCancel=async id=>{state.cancels.push(id??null);if(state.failCancel)throw '停止失败';return true};
    const {emitMock}=await import('/src/lib/events.ts');
    state.started=(id,requestId=state.requestId)=>emitMock('job:started',{jobId:id,requestId});
    state.progress=(id,code='OTHER-001')=>emitMock('library:catalog',{jobId:id,phase,done:1,total:2,code});
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '设置', exact: true }).click()
  await page.getByRole('tab', { name: '名称规则', exact: true }).click()
  await page.getByRole('button', { name: '读取库名称', exact: true }).click()
  if (phase !== 'inventory') {
    await page.locator('[data-catalog-movie]').first().getByRole('checkbox').check()
    await page.getByRole('button', { name: '预览所选影片', exact: true }).click()
    if (phase === 'apply') await page.getByRole('button', { name: '应用名称规则', exact: true }).click()
  }
  await expect.poll(() => page.evaluate(() => typeof (window as any).catalogTask.finish)).toBe('function')
}

async function prepareUndo(page: Page) {
  await prepare(page, 'apply')
  await page.evaluate(async () => {
    const s=(window as any).catalogTask
    s.value={jobId:902,items:[{code:'SMPL-001',dir:'/mock/library/SMPL-001',journalId:'123',error:null}],cancelled:false,indexError:null}
    s.finish()
    const {api}=await import('/src/lib/api.ts')
    api.rollback=()=>new Promise(resolve=>{s.finishUndo=(indexError:string|null)=>resolve({failures:[],status:'rolledBack',indexError})})
  })
  await page.getByRole('button', { name: '撤销本次修改', exact: true }).click()
  await expect.poll(() => page.evaluate(() => typeof (window as any).catalogTask.finishUndo)).toBe('function')
}

test('rollback does not expose a catalog stop button', async ({ page }) => {
  await prepareUndo(page)
  await expect(page.getByRole('button', { name: '停止当前任务', exact: true })).toHaveCount(0)
  await page.evaluate(() => (window as any).catalogTask.finishUndo(null))
  await expect(page.getByRole('status').filter({ hasText: '已撤销本次修改' })).toBeVisible()
})

test('rollback index failure is reported without claiming successful undo', async ({ page }) => {
  await prepareUndo(page)
  await page.evaluate(() => (window as any).catalogTask.finishUndo('索引写入失败'))
  await expect(page.getByRole('alert')).toContainText('索引写入失败')
  await expect(page.getByRole('status').filter({ hasText: '已撤销本次修改' })).toHaveCount(0)
})

for (const phase of ['inventory', 'preview', 'apply'] as const) {
  test(`${phase}: stop before ownership cancels only this request`, async ({ page }) => {
    await prepare(page, phase)
    const stop = page.getByRole('button', { name: '停止当前任务', exact: true })
    await expect(stop).toBeEnabled()
    await stop.click()
    expect(await page.evaluate(() => (window as any).catalogTask.cancels)).toEqual([])
    expect(await page.evaluate(() => typeof (window as any).catalogTask.requestId)).toBe('string')
    await page.evaluate(() => { const s=(window as any).catalogTask;s.started(901,'another-request');s.progress(901) })
    await expect(page.getByRole('status').filter({ hasText: 'OTHER-001' })).toHaveCount(0)
    expect(await page.evaluate(() => (window as any).catalogTask.cancels)).toEqual([])
    await page.evaluate(() => (window as any).catalogTask.started(902))
    await expect.poll(() => page.evaluate(() => (window as any).catalogTask.cancels)).toEqual([902])
    await page.evaluate(() => { const s=(window as any).catalogTask;s.progress(902,'OWN-001');s.progress(903) })
    await expect(page.getByRole('status').filter({ hasText: 'OWN-001' })).toHaveCount(1)
    await expect(page.getByRole('status').filter({ hasText: 'OTHER-001' })).toHaveCount(0)
    await page.evaluate(() => (window as any).catalogTask.finish())
    await expect(stop).toHaveCount(0)
    if (phase === 'preview') await expect(page.getByRole('button', { name: '应用名称规则', exact: true })).toHaveCount(0)
    if (phase === 'apply') await expect(page.getByRole('status').filter({ hasText: '已完成 0 项 · 已取消' })).toBeVisible()
    await page.evaluate(() => (window as any).catalogTask.started(904))
    expect(await page.evaluate(() => (window as any).catalogTask.cancels)).toEqual([902])
  })

  test(`${phase}: busy rejection cannot acquire another catalog job from progress`, async ({ page }) => {
    await prepare(page, phase)
    await page.evaluate(() => (window as any).catalogTask.progress(905))
    await page.getByRole('button', { name: '停止当前任务', exact: true }).click()
    await page.evaluate(() => (window as any).catalogTask.reject('已有任务在运行:catalog-apply'))
    await expect(page.getByRole('alert')).toContainText('已有任务在运行:catalog-apply')
    expect(await page.evaluate(() => (window as any).catalogTask.cancels)).toEqual([])
  })

  test(`${phase}: leaving the page preserves pending targeted cancellation`, async ({ page }) => {
    await prepare(page, phase)
    await page.getByRole('button', { name: '导入整理', exact: true }).click()
    await page.evaluate(() => (window as any).catalogTask.started(906))
    await expect.poll(() => page.evaluate(() => (window as any).catalogTask.cancels)).toEqual([906])
    await page.evaluate(() => (window as any).catalogTask.reject('旧页面失败'))
    await expect(page.getByRole('alert').filter({ hasText: '旧页面失败' })).toHaveCount(0)
    await expect(page.getByRole('button', { name: '添加文件夹', exact: true })).toBeVisible()
  })

  test(`${phase}: cancellation errors keep the task available for retry`, async ({ page }) => {
    await prepare(page, phase)
    await page.evaluate(() => { const s=(window as any).catalogTask;s.started(907);s.progress(907);s.failCancel=true })
    await page.getByRole('button', { name: '停止当前任务', exact: true }).click()
    await expect(page.getByRole('alert')).toContainText('停止失败')
    await page.evaluate(() => (window as any).catalogTask.failCancel=false)
    await page.getByRole('button', { name: '停止当前任务', exact: true }).click()
    expect(await page.evaluate(() => (window as any).catalogTask.cancels)).toEqual([907,907])
    await page.evaluate(() => (window as any).catalogTask.finish())
  })

  test(`${phase}: configuration change retires pending results`, async ({ page }) => {
    await prepare(page, phase)
    await page.evaluate(async () => {
      const {app}=await import('/src/lib/state.svelte.ts')
      app.settings={...app.settings!,library_root:'/mock/another-library'}
    })
    await expect(page.getByRole('button', { name: '停止当前任务', exact: true })).toHaveCount(0)
    await page.evaluate(() => (window as any).catalogTask.started(908))
    await expect.poll(() => page.evaluate(() => (window as any).catalogTask.cancels)).toEqual([908])
    await page.evaluate(() => (window as any).catalogTask.finish())
    await expect(page.locator('[data-catalog-movie]')).toHaveCount(0)
    await expect(page.getByRole('button', { name: '应用名称规则', exact: true })).toHaveCount(0)
    await expect(page.getByRole('status').filter({ hasText: '已完成' })).toHaveCount(0)
  })
}
