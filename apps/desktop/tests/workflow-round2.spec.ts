import { expect, test, type Page } from '@playwright/test'
import { seedLibrary, overrideApi } from './helpers'
test.beforeEach(async ({page})=>seedLibrary(page))
const nav=(page:Page,name:string)=>page.getByRole('navigation',{name:'主导航'}).getByRole('button',{name,exact:true})
async function batch(page:Page) {
  await page.getByRole('button',{name:'媒体库工具',exact:true}).click()
  await page.getByRole('button',{name:'选择影片批量编辑'}).click()
  await expect(page.getByLabel(/^批量选择 /).first()).toBeVisible()
  return page.getByRole('region',{name:'批量收藏与标签'})
}

test('appearance follows system, persists explicit choices and respects reduced motion at narrow widths',async({page})=>{
  await page.emulateMedia({colorScheme:'dark',reducedMotion:'no-preference'})
  await page.goto('/');await nav(page,'设置').click()
  await page.getByLabel('主题',{exact:true}).selectOption('light')
  await page.getByLabel('文字大小',{exact:true}).selectOption('130')
  await page.getByLabel('动态效果',{exact:true}).selectOption('reduced')
  await expect(page.locator('html')).toHaveAttribute('data-theme','light')
  await expect(page.locator('html')).toHaveAttribute('data-motion','reduced')
  expect(await page.locator('body').evaluate(el=>parseFloat(getComputedStyle(el).fontSize))).toBeCloseTo(18.2)
  await page.reload();await nav(page,'设置').click()
  await expect(page.getByLabel('文字大小',{exact:true})).toHaveValue('130')
  for(const width of [960,720,390]){await page.setViewportSize({width,height:800});expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true)}
  await page.getByLabel('主题',{exact:true}).selectOption('system')
  await expect(page.locator('html')).toHaveAttribute('data-theme','dark')
  await page.emulateMedia({colorScheme:'light'})
  await expect(page.locator('html')).toHaveAttribute('data-theme','light')
  await page.screenshot({path:'/tmp/kiroku-round2-appearance.png'})
})

test('settings search locates an unmounted field and preserves draft edits',async({page})=>{
  await page.goto('/');await nav(page,'设置').click()
  await page.getByLabel('搜索设置').fill('目录模板');await page.getByLabel('搜索设置').press('Enter')
  await expect(page.getByLabel('目录模板',{exact:true})).toBeFocused()
  await page.getByLabel('目录模板',{exact:true}).fill('{code}/{title}')
  await page.getByLabel('搜索设置').fill('代理')
  await page.getByRole('button',{name:/全局代理 数据源/}).click()
  await expect(page.locator('#proxy')).toBeFocused()
  await page.getByLabel('搜索设置').fill('目录模板');await page.getByLabel('搜索设置').press('Enter')
  await expect(page.getByLabel('目录模板',{exact:true})).toHaveValue('{code}/{title}')
  await page.getByLabel('搜索设置').fill('没有这样的功能')
  await expect(page.getByText('没有匹配项，试试类别或功能名称。')).toBeVisible()
})

test('pending preview filters and selects files without replacing saved source folders',async({page})=>{
  await overrideApi(page,`const s=await api.getSettings();s.source_dirs=['/mock/source'];await api.saveSettings(s);const scan=api.scanStart;api.scanStart=(dirs,remember)=>{window.selectedScan={dirs,remember};return scan(dirs,remember)};`)
  await page.goto('/');await page.getByRole('tab',{name:/待整理/}).click()
  await expect(page.getByLabel('搜索待整理预览')).toBeVisible()
  await page.getByLabel('识别状态',{exact:true}).selectOption('recognized')
  const row=page.locator('.pending .trow').first(),file=(await row.locator('.name').textContent())!
  await row.getByRole('checkbox').check()
  await page.getByLabel('搜索待整理预览').fill('no matching file')
  await expect(page.getByText('当前预览中没有匹配文件。')).toBeVisible()
  await expect(page.getByText(/已选 1 个（含筛选外文件）/)).toBeVisible()
  await page.getByRole('button',{name:'仅整理所选文件'}).click()
  await expect.poll(()=>page.evaluate(()=>(window as any).selectedScan)).toEqual({dirs:[`/mock/source/${file}`],remember:false})
  await expect(page.getByRole('button',{name:/查找影片信息/})).toBeEnabled()
  expect(await page.evaluate(()=>JSON.parse(localStorage.getItem('kiroku-mock-settings-v2')!).source_dirs)).toEqual(['/mock/source'])
  expect(await page.evaluate(async()=>{const{flow}=await import('/src/lib/importflow.svelte.ts');return flow.items.length})).toBe(1)
})

test('virtual keyboard focus moves across unloaded pages and list mode retains bounded DOM',async({page})=>{
  await page.goto('/');await expect(page.locator('button.pcard').first()).toBeVisible()
  const total=await page.evaluate(async()=>{const{api}=await import('/src/lib/api.ts');return(await api.libraryList({limit:1})).total})
  await page.locator('button.pcard').first().focus();await page.keyboard.press('End')
  await expect(page.locator(`[data-card-index="${total-1}"]`)).toBeFocused()
  await page.keyboard.press('ArrowLeft');await expect(page.locator(`[data-card-index="${total-2}"]`)).toBeFocused()
  await page.keyboard.press('Home');await expect(page.locator('[data-card-index="0"]')).toBeFocused()
  await page.getByLabel('列表视图',{exact:true}).click();await page.locator('button.pcard').first().focus();await page.keyboard.press('End')
  await expect(page.locator(`[data-card-index="${total-1}"]`)).toBeFocused()
  expect(await page.locator('button.pcard').count()).toBeLessThan(30)
  expect(Number(await page.locator('.scroller').getAttribute('data-cached-pages'))).toBeLessThanOrEqual(6)
})

test('overview counts all filtered matches and clears stale results',async({page})=>{
  await page.goto('/');await page.getByRole('button',{name:'媒体库工具',exact:true}).click()
  const overview=page.getByRole('region',{name:'当前筛选概览'})
  await expect(overview.locator('dd').first()).not.toHaveText('0')
  const code=(await page.locator('button.pcard .mono').first().textContent())!
  await page.getByLabel('搜索',{exact:true}).fill(code)
  await expect(overview.locator('dd').first()).toHaveText('1')
  await page.getByLabel('搜索',{exact:true}).fill('impossible-query')
  await expect(overview.locator('dd').first()).toHaveText('0')
  await expect(overview.locator('dd').last()).toHaveText('—')
})

test('bulk favorites preview, apply, paginate selections, undo and expose history details',async({page})=>{
  await page.goto('/');const editor=await batch(page)
  const boxes=page.getByLabel(/^批量选择 /),first=(await boxes.first().getAttribute('aria-label'))!.replace('批量选择 ','')
  await boxes.first().check();await page.getByRole('button',{name:'下一页影片'}).click()
  await expect(boxes.first()).not.toHaveAttribute('aria-label',`批量选择 ${first}`)
  const second=(await boxes.first().getAttribute('aria-label'))!.replace('批量选择 ','');await boxes.first().check()
  await page.getByRole('button',{name:'预览批量修改'}).click();await expect(page.getByLabel('批量编辑差异')).toContainText(first)
  expect(await page.evaluate(async code=>{const{api}=await import('/src/lib/api.ts');return (await api.libraryGet(code))?.favorite},first)).toBe(false)
  await page.getByRole('button',{name:'确认保存批量修改'}).click();await expect(editor).toContainText('已保存 2 部')
  expect(await page.evaluate(async codes=>{const{api}=await import('/src/lib/api.ts');return Promise.all(codes.map(async code=>(await api.libraryGet(code))?.favorite))},[first,second])).toEqual([true,true])
  await page.getByRole('button',{name:'撤销本批修改'}).click();await expect(editor).toContainText('已保存 0 部')
  await nav(page,'历史').click();await page.getByLabel('搜索历史编号').fill(first)
  await page.getByLabel('历史状态',{exact:true}).selectOption('rolledBack')
  await expect(page.getByRole('list',{name:'操作记录'}).locator(':scope > li')).toHaveCount(1)
  await page.getByRole('button',{name:'操作详情',exact:true}).click();await expect(page.getByRole('region',{name:'历史操作详情'})).toContainText('已撤销')
  await page.getByLabel('历史开始日期').fill('2099-01-01');await expect(page.getByText('没有匹配的历史记录')).toBeVisible()
})

test('bulk tags preserve existing values, lock edits, and reject changed disk previews',async({page})=>{
  await page.goto('/');const editor=await batch(page)
  const box=page.getByLabel(/^批量选择 /).first(),code=(await box.getAttribute('aria-label'))!.replace('批量选择 ','');await box.check()
  const before=await page.evaluate(async code=>{const{api}=await import('/src/lib/api.ts');return (await api.libraryGet(code))!.meta},code)
  await page.getByLabel('批量编辑操作').selectOption('tags');await page.getByLabel('批量添加标签').fill('工作集\n工作集')
  await page.getByRole('button',{name:'预览批量修改'}).click();await expect(page.getByLabel('批量编辑差异')).toContainText('工作集')
  await page.getByRole('button',{name:'确认保存批量修改'}).click();await expect(editor).toContainText('已保存 1 部')
  const after=await page.evaluate(async code=>{const{api}=await import('/src/lib/api.ts');return (await api.libraryGet(code))!.meta},code)
  expect(after.tags).toEqual([...before.tags,'工作集']);expect(after.locked_fields).toContain('tags')
  const result=await page.evaluate(async code=>{const{api}=await import('/src/lib/api.ts');const p=await api.batchEditPreview([code],{kind:'tags',add:[],remove:['工作集']});const changed=await api.revisionPreview(code,{title:'外部修订'});await api.revisionSave(changed.token,changed.root,code);return api.batchEditApply(p.token,p.root)},code)
  expect(result.results).toHaveLength(0);expect(result.failed[0]).toContain('内容已变化')
})

test('batch cancellation retains completed journals and isolates settings changes',async({page})=>{
  await page.goto('/')
  const result=await page.evaluate(async()=>{
    const{api}=await import('/src/lib/api.ts');const{listen}=await import('/src/lib/events.ts')
    const codes=(await api.libraryList({limit:20})).items.map(m=>m.code)
    const p=await api.batchEditPreview(codes,{kind:'favorite',favorite:true})
    const stop=await listen('batch:edit',event=>{if(event.completed===1)void api.jobCancel(event.jobId)})
    const result=await api.batchEditApply(p.token,p.root);stop()
    const changed=await api.batchEditPreview([codes[1]],{kind:'favorite',favorite:true})
    const s=await api.getSettings();s.notify_on_finish=!s.notify_on_finish;await api.saveSettings(s)
    let error='';try{await api.batchEditApply(changed.token,changed.root)}catch(e){error=String(e)}
    return {result,error,history:(await api.historyList()).items}
  })
  expect(result.result.cancelled).toBe(true);expect(result.result.results).toHaveLength(1);expect(result.history).toHaveLength(1);expect(result.error).toContain('预览已过期')
})


test('large text batch tools and pending controls fit narrow windows',async({page})=>{
  await page.addInitScript(()=>localStorage.setItem('kiroku-appearance-v1',JSON.stringify({theme:'dark',textScale:130,motion:'reduced'})))
  await overrideApi(page,`const s=await api.getSettings();s.source_dirs=['/mock/source'];await api.saveSettings(s);`)
  await page.goto('/');await batch(page)
  await page.getByLabel('批量编辑操作').selectOption('tags')
  for(const width of [960,720,390]) {
    await page.setViewportSize({width,height:900})
    const overflow=await page.locator('.utility-drawer, .history-filters, .view-shell').evaluateAll(els=>els.filter(el=>el.scrollWidth>el.clientWidth+1).map(el=>el.className))
    expect(overflow).toEqual([])
    await expect(page.getByLabel('批量添加标签')).toBeVisible()
  }
  await page.getByLabel('批量添加标签').fill('工作集')
  await page.screenshot({path:'/tmp/kiroku-round2-batch.png'})
  await page.getByRole('button',{name:'媒体库工具',exact:true}).click()
  await page.getByRole('tab',{name:/待整理/}).click()
  await expect(page.getByLabel('待整理排序')).toBeVisible()
  expect(await page.locator('.pending').evaluate(el=>el.scrollWidth<=el.clientWidth+1)).toBe(true)
  await page.screenshot({path:'/tmp/kiroku-round2-pending.png'})
})

test('invalid history dates do not leave a false loading state and clearing them recovers',async({page})=>{
  await page.goto('/');await nav(page,'历史').click()
  await page.getByLabel('历史开始日期').fill('2026-10-29');await page.getByLabel('历史结束日期').fill('2026-10-01')
  await expect(page.getByText('结束日期不能早于开始日期。')).toBeVisible()
  await expect(page.getByText('调整日期范围后会自动重新查询。')).toBeVisible()
  await page.getByRole('button',{name:'清除历史筛选'}).click()
  await expect(page.getByText('还没有操作记录')).toBeVisible()
})


test('overview serializes slow native calls and discards obsolete queued queries',async({page})=>{
  await overrideApi(page,`const list=api.libraryList;api.libraryList=q=>{window.latestListText=q.text;return list(q)};const get=api.libraryOverview;window.overviewCalls=[];api.libraryOverview=q=>{window.overviewCalls.push(q);return new Promise(resolve=>{window.releaseOverview=async()=>resolve(await get(q))})};`)
  await page.goto('/');await page.getByRole('button',{name:'媒体库工具',exact:true}).click()
  await expect.poll(()=>page.evaluate(()=>(window as any).overviewCalls.length)).toBe(1)
  await page.getByLabel('搜索',{exact:true}).fill('first query')
  await expect(page.locator('button.pcard')).toHaveCount(0)
  await page.getByLabel('搜索',{exact:true}).fill('last query')
  await expect.poll(()=>page.evaluate(()=>(window as any).latestListText)).toBe('last query')
  expect(await page.evaluate(()=>(window as any).overviewCalls.length)).toBe(1)
  await page.evaluate(()=>(window as any).releaseOverview())
  await expect.poll(()=>page.evaluate(()=>(window as any).overviewCalls.length)).toBe(2)
  expect(await page.evaluate(()=>(window as any).overviewCalls[1].text)).toBe('last query')
  await page.evaluate(()=>(window as any).releaseOverview())
  await expect(page.getByRole('region',{name:'当前筛选概览'}).locator('dd').first()).toHaveText('0')
})

test('invalid date selection cancels an active slow history read and clears busy controls',async({page})=>{
  await overrideApi(page,`window.historyStarted=0;window.historyStops=[];api.historyList=()=>{window.historyStarted++;return new Promise(()=>{})};api.historyCancel=async id=>{window.historyStops.push(id);return true};`)
  await page.goto('/');await nav(page,'历史').click()
  await expect.poll(()=>page.evaluate(()=>(window as any).historyStarted)).toBe(1)
  await page.getByLabel('历史开始日期').fill('2026-10-29');await page.getByLabel('历史结束日期').fill('2026-10-01')
  await expect(page.getByRole('button',{name:'停止读取',exact:true})).toHaveCount(0)
  await expect.poll(()=>page.evaluate(()=>(window as any).historyStops.length)).toBeGreaterThan(0)
  await page.getByRole('button',{name:'清除历史筛选'}).click()
  await expect.poll(()=>page.evaluate(()=>(window as any).historyStarted)).toBe(2)
})
