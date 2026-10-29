import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'
test('optional catalog sources preserve routes and require explicit censored route addition', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('kiroku-mock-settings-v2', JSON.stringify({ library_root:'/mock/library', sources:[{id:'javdb',enabled:true,base:'https://javdb.com'}], routes:{censored:['javdb'],amateur:['javdb'],uncensored:['javdb'],fc2:['javdb']}, field_priority:{title:['javdb']} })))
  await page.goto('/')
  await page.getByRole('navigation', { name:'主导航' }).getByRole('button', { name:'设置', exact:true }).click()
  await page.getByRole('tab', { name:'数据源', exact:true }).click()
  for (const [id,name] of [['javlibrary','JavLibrary'],['xcity','XCity']]) {
    const checkbox=page.locator('label.switch').filter({hasText:name}).getByRole('checkbox')
    await expect(checkbox).not.toBeChecked()
    await checkbox.check()
    await page.getByLabel(`${name} 域名`).fill(`https://${id}.example.test`)
    await page.locator(`#cookie-${id}`).fill('session=local-secret')
    await page.getByRole('button',{name:'加入支持类型路线',exact:true}).last().click()
  }
  await page.getByRole('button',{name:/保存更改/}).click()
  await expect(page.getByRole('button',{name:'已保存',exact:true})).toBeVisible()
  const saved=await page.evaluate(()=>JSON.parse(localStorage.getItem('kiroku-mock-settings-v2')!))
  expect(saved.routes.censored).toEqual(['javdb','javlibrary','xcity'])
  for (const type of ['amateur','uncensored','fc2']) expect(saved.routes[type]).toEqual(['javdb'])
  expect(saved.field_priority.title).toEqual(['javdb'])
  for(const id of ['javlibrary','xcity']) {expect(saved.sources.find((s:any)=>s.id===id).cookie).toBe('session=local-secret');expect(saved.sources.find((s:any)=>s.id===id).base).toBe(`https://${id}.example.test`)}
})

test('mock source self-check distinguishes disabled or unknown sources',async({page})=>{
  await page.goto('/')
  const result=await page.evaluate(async()=>{
    const {mockApi}=await import('/src/lib/mock/engine.ts')
    return await mockApi.sourcesCheck(['javlibrary','xcity','unknown-catalog-source'])
  })
  expect(result.map(c=>c.outcome.reason)).toEqual(['disabledOrUnknown','disabledOrUnknown','disabledOrUnknown'])
  expect(result.every(c=>c.outcome.status==='skipped')).toBe(true)
})


test('new source snapshots support field choice and remain isolated from uncensored routes',async({page})=>{
  await seedLibrary(page)
  await overrideApi(page,"window.catalogApi=api; window.catalogMock=(await import('/src/lib/mock/engine.ts')).mockApi;")
  await page.goto('/')
  await page.locator('button.pcard').first().click()
  const code=await page.getByLabel('影片详情',{exact:true}).locator('.head .code').innerText()
  const result=await page.evaluate(async code=>{
    const api=(window as any).catalogApi
    const settings=await api.getSettings()
    for(const source of settings.sources) source.enabled=['javlibrary','xcity'].includes(source.id)
    settings.routes.censored=['javlibrary','xcity']
    await api.saveSettings(settings)
    let preview=await api.metadataUpdatePreview(code,{kind:'refresh',fields:[],replace:false,force:true})
    await api.revisionSave(preview.token,preview.root,code)
    const candidates=(await api.metadataSourceCandidates(code)).title.map((c:any)=>c.source)
    preview=await api.metadataUpdatePreview(code,{kind:'source',field:'title',source:'xcity'})
    await api.revisionSave(preview.token,preview.root,code)
    const selected=(await api.libraryGet(code)).meta.provenance.title
    const mock=(window as any).catalogMock
    const original=mock.libraryGet
    mock.libraryGet=async(id:string)=>{const detail=await original(id);detail.contentType='uncensored';for(const snapshot of Object.values(detail.meta.raw_sources) as any[])snapshot.content_type='uncensored';return detail}
    settings.routes.uncensored=['javlibrary','xcity']
    await api.saveSettings(settings)
    const wrongCandidates=(await api.metadataSourceCandidates(code)).title
    let error=''
    try{await api.metadataUpdatePreview(code,{kind:'refresh',fields:[],replace:false,force:true})}catch(e){error=String(e)}
    mock.libraryGet=original
    return {candidates,selected,wrongCandidates,error}
  },code)
  expect(result.candidates).toEqual(['javlibrary','xcity'])
  expect(result.selected).toBe('xcity')
  expect(result.wrongCandidates).toEqual([])
  expect(result.error).toContain('没有可用的数据源')
})

test('AVBase is opt-in, joins censored and amateur routes and goes through the solver', async ({ page }) => {
  await seedLibrary(page)
  await page.goto('/')
  await page.getByRole('navigation', { name:'主导航' }).getByRole('button', { name:'设置', exact:true }).click()
  await page.getByRole('tab', { name:'数据源', exact:true }).click()
  const checkbox = page.locator('label.switch').filter({ hasText: 'AVBase' }).getByRole('checkbox').first()
  await expect(checkbox).not.toBeChecked()
  await checkbox.check()
  await page.getByRole('button', { name: 'AVBase 说明', exact: true }).hover()
  await expect(page.getByText(/直连会被 Cloudflare 挑战拦住，需要求解器/)).toBeVisible()
  await page.getByRole('button', { name: '加入支持类型路线', exact: true }).last().click()
  await page.getByLabel('求解器地址').fill('http://127.0.0.1:8191/v1')
  await page.getByLabel('AVBase 经求解器访问').check()
  await page.getByRole('button', { name: /保存更改/ }).click()
  await expect(page.getByRole('button', { name: '已保存', exact: true })).toBeVisible()
  const saved = await page.evaluate(() => JSON.parse(localStorage.getItem('kiroku-mock-settings-v2')!))
  expect(saved.routes.censored.at(-1)).toBe('avbase')
  expect(saved.routes.amateur.at(-1)).toBe('avbase')
  expect(saved.routes.uncensored).not.toContain('avbase')
  expect(saved.sources.find((s: { id: string }) => s.id === 'avbase')).toMatchObject({ enabled: true, solver: true })
})
