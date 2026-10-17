import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

test('combined UI edit, cache refresh, two crops and reverse history restore stable files', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, 'window.testApi=api; window.playCalls=[]; api.play=async code=>window.playCalls.push(code);')
  await page.goto('/')
  await page.locator('button.pcard').first().click()
  const panel = page.getByLabel('影片详情', { exact: true })
  await expect(panel).toBeVisible()
  expect(await page.evaluate(() => (window as any).playCalls)).toEqual([])
  const code = await panel.locator('.head .code').innerText()
  const files = await panel.locator('.files').innerText()
  const before = await page.evaluate(async code => (await (window as any).testApi.libraryGet(code)).meta, code)
  await panel.getByRole('button', { name: '编辑元数据', exact: true }).click()
  await panel.getByLabel('编辑标题', { exact: true }).fill('集成人工标题')
  await panel.getByLabel('编辑系列', { exact: true }).fill('')
  await panel.getByRole('button', { name: '预览差异', exact: true }).click()
  await panel.getByRole('button', { name: '确认保存', exact: true }).click()
  await expect(panel.locator('h2')).toHaveText('集成人工标题')
  const result = await page.evaluate(async code => {
    const api=(window as any).testApi
    const lock=await api.metadataUpdatePreview(code,{kind:'lock',field:'series',locked:true})
    await api.revisionSave(lock.token,lock.root,code)
    const cached=await api.metadataUpdatePreview(code,{kind:'refresh',fields:[],replace:false,force:false})
    await api.revisionSave(cached.token,cached.root,code)
    const fresh=await api.metadataUpdatePreview(code,{kind:'refresh',fields:['plot','title','series'],replace:true,force:true})
    await api.revisionSave(fresh.token,fresh.root,code)
    return (await api.libraryGet(code)).meta
  },code)
  expect(result.title).toBe('集成人工标题'); expect(result.series).toBeNull()
  expect(result.studio).toBe(before.studio); expect(result.tags).toEqual(before.tags)
  for (const steps of [5,3]) {
    await panel.getByRole('button',{name:'裁剪海报',exact:true}).click()
    const crop=page.getByRole('dialog',{name:'裁剪海报'})
    await crop.getByRole('slider',{name:'裁剪框位置'}).focus()
    for (let i=0;i<steps;i++) await page.keyboard.press('Shift+ArrowLeft')
    await crop.getByRole('button',{name:'保存海报',exact:true}).click()
    await expect(crop).toHaveCount(0)
  }
  await expect(panel.locator('.files')).toHaveText(files)
  const restored = await page.evaluate(async code => {
    const api=(window as any).testApi
    const history=(await api.historyList(undefined,{limit:1000})).items.filter((h:any)=>h.id.startsWith('revision-'))
    const results=[]
    for(const h of history)results.push(await api.rollback(h.id))
    return {results,meta:(await api.libraryGet(code)).meta}
  },code)
  expect(restored.results).toHaveLength(6)
  expect(restored.results.every((r:any)=>r.failures.length===0)).toBe(true)
  expect(restored.meta).toEqual(before)
  expect(await page.evaluate(() => (window as any).playCalls)).toEqual([])
})
