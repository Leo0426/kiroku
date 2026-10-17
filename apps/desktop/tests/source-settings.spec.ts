import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

test('old settings gain disabled sources; route addition is explicit and persists with API endpoint', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('kiroku-mock-settings-v2', JSON.stringify({
    library_root: '/mock/library', sources: [{ id: 'javdb', enabled: true, base: 'https://javdb.com' }, { id: 'javbus', enabled: true, base: 'https://www.javbus.com' }, { id: 'jav321', enabled: true, base: 'https://www.jav321.com' }],
    routes: { censored: ['javdb'], uncensored: ['javbus'], fc2: ['javdb'] },
    field_priority: { title: ['jav321'] },
  })))
  await page.goto('/')
  await page.getByRole('navigation', { name: '主导航' }).getByRole('button', { name: '设置', exact: true }).click()
  await page.getByRole('tab', { name: '数据源', exact: true }).click()
  const dmm = page.locator('label.switch').filter({ hasText: 'DMM / FANZA 数字影片' }).getByRole('checkbox')
  await expect(dmm).not.toBeChecked()
  await dmm.check()
  await expect(page.getByText('尚未加入任何路线，启用后仍不会请求。')).toBeVisible()
  await page.getByLabel('数字影片 API').fill('https://example.test/graphql')
  await page.getByRole('button', { name: '加入支持类型路线', exact: true }).click()
  await expect(page.getByText('已加入路线：有码')).toBeVisible()
  await page.getByRole('button', { name: /保存更改/ }).click()
  await expect(page.getByRole('button', { name: '已保存', exact: true })).toBeVisible()
  const saved = await page.evaluate(() => JSON.parse(localStorage.getItem('kiroku-mock-settings-v2')!))
  expect(saved.routes.censored).toEqual(['javdb', 'dmm'])
  expect(saved.routes.uncensored).toEqual(['javbus'])
  expect(saved.routes.fc2).toEqual(['javdb'])
  expect(saved.field_priority.title).toEqual(['jav321'])
  expect(saved.sources.find((s: { id: string }) => s.id === 'dmm').api_base).toBe('https://example.test/graphql')
  for (const id of ['mgstage', 'fc2', 'fc2ppvdb', 'avsox']) expect(saved.sources.find((s: { id: string }) => s.id === id).enabled).toBe(false)
})


test('mock refresh and candidates reject wrong source types while valid old and DMM routes still work', async ({ page }) => {
  await seedLibrary(page)
  await overrideApi(page, "window.testApi = api; window.testMockApi = (await import('/src/lib/mock/engine.ts')).mockApi;")
  await page.goto('/')
  await page.locator('button.pcard').first().click()
  const code = await page.getByLabel('影片详情', { exact: true }).locator('.head .code').innerText()
  const result = await page.evaluate(async code => {
    const api = (window as any).testApi
    const settings = await api.getSettings()
    for (const source of settings.sources) source.enabled = true
    settings.routes.censored = ['javdb', 'dmm', 'fc2', 'fc2ppvdb', 'mgstage', 'avsox']
    await api.saveSettings(settings)
    const preview = await api.metadataUpdatePreview(code, {kind:'refresh',fields:[],replace:false,force:true})
    await api.revisionSave(preview.token, preview.root, code)
    const snapshots = Object.keys((await api.libraryGet(code)).meta.raw_sources)
    const candidates = (await api.metadataSourceCandidates(code)).title.map((c: any) => c.source)
    settings.routes.censored = ['fc2', 'fc2ppvdb', 'mgstage', 'avsox']
    await api.saveSettings(settings)
    let emptyRouteError = ''
    try { await api.metadataUpdatePreview(code, {kind:'refresh',fields:[],replace:false,force:true}) } catch (error) { emptyRouteError = String(error) }
    const wrongCandidates = (await api.metadataSourceCandidates(code)).title
    // Synthetic library type changes exercise the same declared type gate as desktop;
    // these are mock metadata, not claimed site response fixtures.
    const mock = (window as any).testMockApi
    const libraryGet = mock.libraryGet
    mock.libraryGet = async (id: string) => { const detail = await libraryGet(id); detail.contentType='fc2'; detail.meta.raw_sources.dmm.content_type='fc2'; return detail }
    settings.routes.fc2 = ['dmm']
    await api.saveSettings(settings)
    let dmmFc2Error = ''
    try { await api.metadataUpdatePreview(code, {kind:'refresh',fields:[],replace:false,force:true}) } catch (error) { dmmFc2Error = String(error) }
    const dmmFc2Candidates = (await api.metadataSourceCandidates(code)).title
    mock.libraryGet = async () => ({...(await libraryGet(code)), contentType:'censored'})
    settings.routes.censored = ['dmm']
    await api.saveSettings(settings)
    let dmmFormatError = ''
    try { await api.metadataUpdatePreview('T38-068', {kind:'refresh',fields:[],replace:false,force:true}) } catch (error) { dmmFormatError = String(error) }
    mock.libraryGet = libraryGet
    return {snapshots, candidates, wrongCandidates, emptyRouteError, dmmFc2Error, dmmFc2Candidates, dmmFormatError}
  }, code)
  expect(result.snapshots).toEqual(['javdb', 'dmm'])
  expect(result.candidates).toEqual(['javdb', 'dmm'])
  expect(result.wrongCandidates).toEqual([])
  expect(result.emptyRouteError).toContain('没有可用的数据源')
  expect(result.dmmFc2Error).toContain('没有可用的数据源')
  expect(result.dmmFc2Candidates).toEqual([])
  expect(result.dmmFormatError).toContain('没有可用的数据源')
})

test('each source can follow the global proxy, connect directly or use its own proxy', async ({ page }) => {
  await seedLibrary(page)
  await page.goto('/')
  await page.getByRole('navigation', { name: '主导航' }).getByRole('button', { name: '设置', exact: true }).click()
  await page.getByRole('tab', { name: '数据源', exact: true }).click()
  await page.locator('label.switch').filter({ hasText: 'DMM / FANZA 数字影片' }).getByRole('checkbox').check()
  await page.getByLabel('DMM / FANZA 数字影片 代理').selectOption('custom')
  await page.getByLabel('DMM / FANZA 数字影片 代理地址').fill('http://127.0.0.1:7891')
  await page.getByLabel('JavDB 代理', { exact: true }).selectOption('direct')
  await page.getByRole('button', { name: /保存更改/ }).click()
  await expect(page.getByRole('button', { name: '已保存', exact: true })).toBeVisible()
  const saved = await page.evaluate(() => JSON.parse(localStorage.getItem('kiroku-mock-settings-v2')!))
  const src = (id: string) => saved.sources.find((s: { id: string }) => s.id === id)
  expect(src('dmm').proxy).toEqual({ mode: 'custom', url: 'http://127.0.0.1:7891' })
  expect(src('javdb').proxy).toEqual({ mode: 'direct' })
  expect(src('jav321').proxy).toBeUndefined()

  await page.getByLabel('DMM / FANZA 数字影片 代理地址').fill('')
  await page.getByRole('button', { name: /保存更改/ }).click()
  await expect(page.getByRole('alert')).toContainText('自定义代理地址')
})

test('a challenged source can be routed through the user solver once its address is set', async ({ page }) => {
  await seedLibrary(page)
  await page.goto('/')
  await page.getByRole('navigation', { name: '主导航' }).getByRole('button', { name: '设置', exact: true }).click()
  await page.getByRole('tab', { name: '数据源', exact: true }).click()
  await page.locator('label.switch').filter({ hasText: 'JavLibrary' }).getByRole('checkbox').first().check()
  // 没填地址时不出现开关
  await expect(page.getByLabel('JavLibrary 经求解器访问')).toHaveCount(0)
  await page.getByLabel('求解器地址').fill('http://127.0.0.1:8191/v1')
  await page.getByLabel('JavLibrary 经求解器访问').check()
  await expect(page.getByLabel('DMM / FANZA 数字影片 经求解器访问')).toHaveCount(0)
  await page.getByRole('button', { name: /保存更改/ }).click()
  await expect(page.getByRole('button', { name: '已保存', exact: true })).toBeVisible()
  const saved = await page.evaluate(() => JSON.parse(localStorage.getItem('kiroku-mock-settings-v2')!))
  expect(saved.solver).toEqual({ url: 'http://127.0.0.1:8191/v1', timeout_secs: 60 })
  expect(saved.sources.find((s: { id: string }) => s.id === 'javlibrary').solver).toBe(true)
  expect(saved.sources.find((s: { id: string }) => s.id === 'javdb').solver).toBeUndefined()

  await page.getByLabel('求解器地址').fill('127.0.0.1:8191')
  await page.getByRole('button', { name: /保存更改/ }).click()
  await expect(page.getByRole('alert', { name: '保存失败' })).toContainText('求解器地址')
})
