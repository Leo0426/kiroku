import { expect, test, type Page } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

type Kind = 'targets' | 'videos' | 'append-preview' | 'append-apply' | 'export-preview' | 'export-apply' | 'metadata' | 'artwork-open' | 'artwork-preview' | 'artwork-save' | 'picker' | 'candidates' | 'list' | 'undo'
const stopLabel = (kind: Kind) => kind.startsWith('export') ? '取消导出任务' : kind === 'metadata' ? '取消更新任务' : '取消追加任务'

async function prepare(page: Page, kind: Kind) {
  await seedLibrary(page)
  await overrideApi(page, `
    const kind=${JSON.stringify(kind)};
    const state=window.editor={cancels:[],requestId:null,videoCalls:0,saves:0};
    const pending=(requestId,value)=>{state.requestId=requestId;return new Promise((resolve,reject)=>{state.finish=()=>resolve(value);state.reject=reject})};
    api.jobCancel=async id=>{state.cancels.push(id??null);return true};
    const {emitMock}=await import('/src/lib/events.ts');
    state.started=(id,requestId=state.requestId)=>emitMock('job:started',{jobId:id,requestId});
    state.progress=(jobId,completed)=>emitMock('nfo:export',{jobId,completed,failed:0});
    api.pickMediaFiles=async()=>['/mock/extra.mp4'];
    const videos=api.appendTargetVideos;
    api.appendTargetVideos=(...args)=>{state.videoCalls++;return kind==='videos'?pending(args[2],[]):videos(...args)};
    if(kind==='targets')api.appendTargets=(_code,id)=>pending(id,['/mock/library/movie']);
    if(kind==='append-preview')api.appendPreview=(code,inputs,target,video,id)=>pending(id,{token:'owned',root:'/mock/library',code,targetDir:target,ops:[{CopyFile:{from:inputs[0],to:target+'/extra.mp4'}}],skipped:[],conflicts:[]});
    if(kind==='append-apply')api.appendApply=(_token,id)=>pending(id,{journalId:'owned',completed:0,skipped:[],failed:[],cancelled:true});
    if(kind==='export-preview')api.nfoExportPreview=(_codes,_language,id)=>pending(id,{token:'owned',root:'/mock/library',items:[]});
    if(kind==='export-apply')api.nfoExportApply=(_token,_root,id)=>pending(id,{jobId:902,results:[],failed:[],cancelled:true});
    if(kind==='metadata')api.metadataUpdatePreview=async(code,_update,id)=>{const d=await api.libraryGet(code);return pending(id,{token:'owned',root:'/mock/library',code,before:d.meta,after:d.meta,changes:[]})};
    if(kind==='artwork-open')api.artworkOpen=(_code,id)=>pending(id,null);
    if(kind==='artwork-preview')api.artworkPreview=(_token,root,code,_crop,id)=>pending(id,{revision:{token:'owned',root,code}});
    const save=api.revisionSave;api.revisionSave=(...args)=>{state.saves++;return kind==='artwork-save'?pending(null,{error:null,indexError:null}):save(...args)};
    if(kind==='picker')api.pickMediaFiles=()=>pending(null,[]);
    if(kind==='candidates')api.metadataSourceCandidates=()=>pending(null,{});
    if(kind==='undo')api.rollback=()=>pending(null,{failures:[],indexError:null});
    if(kind==='list'){const list=api.libraryList;api.libraryList=q=>q.limit===200?pending(null,{items:[],total:0}):list(q)};
  `)
  await page.goto('/')
  if (kind.startsWith('export') || kind === 'list') {
    await page.getByRole('button', { name: '媒体库工具', exact: true }).click()
    await page.getByRole('button', { name: '批量导出 NFO 语言', exact: true }).click()
    if (kind !== 'list') {
      await page.getByLabel('批量 NFO 语言导出', { exact: true }).getByRole('checkbox').first().check()
      await page.getByRole('button', { name: '预览批量 NFO 导出', exact: true }).click()
      if (kind === 'export-apply') await page.getByRole('button', { name: '确认批量导出', exact: true }).click()
    }
  } else {
    await page.locator('button.pcard').first().click()
    if (kind.startsWith('artwork')) {
      await page.getByRole('button', { name: '裁剪海报', exact: true }).click()
      if (kind !== 'artwork-open') {
        await expect(page.getByAltText('裁剪原图')).toBeVisible()
        await page.getByRole('button', { name: '保存海报', exact: true }).click()
      }
    } else if (kind === 'metadata' || kind === 'candidates') {
      await page.getByRole('button', { name: '更新与来源', exact: true }).click()
      if (kind === 'metadata') await page.getByRole('button', { name: '预览更新', exact: true }).click()
    } else {
      await page.getByRole('button', { name: '追加视频或字幕', exact: true }).click()
      if (kind !== 'targets' && kind !== 'videos') {
        await page.getByRole('button', { name: '选择文件', exact: true }).click()
        if (kind !== 'picker') {
          await page.getByRole('button', { name: '预览追加', exact: true }).click()
          if (kind === 'append-apply' || kind === 'undo') await page.getByRole('button', { name: '确认追加', exact: true }).click()
          if (kind === 'undo') await page.getByRole('button', { name: '撤销本次追加', exact: true }).click()
        }
      }
    }
  }
  await expect.poll(() => page.evaluate(() => typeof (window as any).editor.finish)).toBe('function')
}

const cancellableKinds = ['targets','videos','append-preview','append-apply','export-preview','export-apply','metadata'] as const
for (const kind of cancellableKinds) {
  test(`${kind}: early cancellation waits for ownership and does not accept cancelled previews`, async ({ page }) => {
    await prepare(page, kind)
    await page.getByRole('button', { name: stopLabel(kind), exact: true }).click()
    expect(await page.evaluate(() => (window as any).editor.cancels)).toEqual([])
    expect(await page.evaluate(() => typeof (window as any).editor.requestId)).toBe('string')
    await page.evaluate(() => (window as any).editor.started(901,'unrelated'))
    expect(await page.evaluate(() => (window as any).editor.cancels)).toEqual([])
    await page.evaluate(() => (window as any).editor.started(902))
    await expect.poll(() => page.evaluate(() => (window as any).editor.cancels)).toEqual([902])
    await page.evaluate(() => { const s=(window as any).editor;s.started(903);s.finish() })
    await expect(page.getByRole('button', { name: stopLabel(kind), exact: true })).toHaveCount(0)
    await expect(page.getByRole('button', { name: /^(确认追加|确认批量导出|确认更新)$/ })).toHaveCount(0)
    if (kind === 'targets') expect(await page.evaluate(() => (window as any).editor.videoCalls)).toBe(0)
    await page.evaluate(() => (window as any).editor.started(904))
    expect(await page.evaluate(() => (window as any).editor.cancels)).toEqual([902])
  })
  test(`${kind}: configuration change keeps a pending scoped stop and discards late results`, async ({ page }) => {
    await prepare(page, kind)
    await page.evaluate(async () => { const {app}=await import('/src/lib/state.svelte.ts');app.settings={...app.settings!,min_interval_secs:7} })
    await expect(page.getByRole('button', { name: stopLabel(kind), exact: true })).toHaveCount(0)
    await page.evaluate(() => (window as any).editor.started(905))
    await expect.poll(() => page.evaluate(() => (window as any).editor.cancels)).toEqual([905])
    await page.evaluate(() => (window as any).editor.reject('旧请求失败'))
    await expect(page.getByRole('alert').filter({hasText:'旧请求失败'})).toHaveCount(0)
  })
}
for (const kind of ['targets','export-preview','metadata'] as const) {
  test(`${kind}: rejected startup leaves the occupied slot alone`, async ({ page }) => {
    await prepare(page,kind)
    await page.getByRole('button',{name:stopLabel(kind),exact:true}).click()
    await page.evaluate(()=>(window as any).editor.reject('已有任务在运行:repair'))
    await expect(page.getByRole('alert')).toContainText('已有任务在运行:repair')
    expect(await page.evaluate(()=>(window as any).editor.cancels)).toEqual([])
  })
}
for (const kind of ['artwork-open','artwork-preview'] as const) {
  test(`${kind}: destroying the dialog only cancels its owned stage and never saves a late preview`, async ({ page }) => {
    await prepare(page,kind)
    if(kind==='artwork-open') await page.getByRole('button',{name:'关闭裁剪',exact:true}).click()
    else await page.evaluate(async()=>{const {app}=await import('/src/lib/state.svelte.ts');app.view='import'})
    await expect(page.getByRole('dialog',{name:'裁剪海报'})).toHaveCount(0)
    expect(await page.evaluate(()=>(window as any).editor.cancels)).toEqual([])
    await page.evaluate(()=>(window as any).editor.started(906))
    await expect.poll(()=>page.evaluate(()=>(window as any).editor.cancels)).toEqual([906])
    await page.evaluate(()=>(window as any).editor.finish())
    expect(await page.evaluate(()=>(window as any).editor.saves)).toBe(0)
  })
}
test('artwork commit cannot be cancelled on unmount',async({page})=>{
  await prepare(page,'artwork-save')
  await expect(page.getByRole('button',{name:'关闭裁剪',exact:true})).toBeDisabled()
  await page.evaluate(async()=>{const {app}=await import('/src/lib/state.svelte.ts');app.view='import'})
  await expect(page.getByRole('dialog',{name:'裁剪海报'})).toHaveCount(0)
  expect(await page.evaluate(()=>(window as any).editor.cancels)).toEqual([])
  await page.evaluate(()=>(window as any).editor.finish())
})
for(const [kind,label] of [['picker','取消追加任务'],['undo','取消追加任务'],['candidates','取消更新任务'],['list','取消导出任务']] as const){
  test(`${kind}: noncancellable work has no stop control`,async({page})=>{
    await prepare(page,kind)
    await expect(page.getByRole('button',{name:label,exact:true})).toHaveCount(0)
    await page.evaluate(()=>(window as any).editor.finish())
  })
}
test('export progress belongs to the owned apply job',async({page})=>{
  await prepare(page,'export-apply')
  const batch=page.getByLabel('批量 NFO 语言导出',{exact:true})
  await page.evaluate(()=>{const s=(window as any).editor;s.progress(800,99);s.started(902);s.progress(902,2);s.progress(800,100)})
  await expect(batch.getByRole('status')).toHaveText('已导出 2 部，失败 0 部')
  await expect(batch.getByRole('button',{name:'批量导出 NFO 语言',exact:true})).toBeDisabled()
  await page.evaluate(()=>(window as any).editor.finish())
})

test('mock editor commands announce ownership and cancellation leaves media and history unchanged',async({page})=>{
  await seedLibrary(page);await page.goto('/')
  const result=await page.evaluate(async()=>{
    const {api}=await import('/src/lib/api.ts'),{listen}=await import('/src/lib/events.ts')
    const code=(await api.libraryList({limit:1})).items[0].code
    const before=await api.libraryGet(code),historyBefore=await api.historyList()
    const original=await api.artworkOpen(code)
    const append=await api.appendPreview(code,['/mock/extra.mp4'],before!.dir)
    const batch=await api.nfoExportPreview([code],'original')
    const requests:string[]=[],errors:string[]=[]
    const stop=await listen('job:started',e=>{requests.push(e.requestId);void api.jobCancel(e.jobId)})
    const appended=await api.appendApply(append.token,'append-apply')
    const exported=await api.nfoExportApply(batch.token,batch.root,'export-apply')
    const reads=[
      ()=>api.appendTargets(code,'targets'),
      ()=>api.appendTargetVideos(code,before!.dir,'videos'),
      ()=>api.appendPreview(code,['/mock/extra.mp4'],before!.dir,undefined,'append-preview'),
      ()=>api.nfoExportPreview([code],'original','export-preview'),
      ()=>api.metadataUpdatePreview(code,{kind:'lock',field:'title',locked:true},'metadata'),
      ()=>api.artworkOpen(code,'artwork-open'),
      ()=>api.artworkPreview(original.token,original.root,code,{x:0,y:0,width:100,height:100},'artwork-preview'),
    ]
    for(const read of reads){try{await read();errors.push('unexpected success')}catch(e){errors.push(String(e))}}
    stop()
    return {requests,errors,appended,exported,sameMedia:JSON.stringify(before)===JSON.stringify(await api.libraryGet(code)),sameHistory:JSON.stringify(historyBefore)===JSON.stringify(await api.historyList())}
  })
  expect(result.requests).toEqual(['append-apply','export-apply','targets','videos','append-preview','export-preview','metadata','artwork-open','artwork-preview'])
  expect(result.errors).toHaveLength(7)
  for(const error of result.errors)expect(error).toContain('取消')
  expect(result.appended).toMatchObject({completed:0,cancelled:true})
  expect(result.exported).toMatchObject({results:[],cancelled:true})
  expect(result.sameMedia).toBe(true);expect(result.sameHistory).toBe(true)
})

test('batch export cancellation keeps the completed film and its undo journal',async({page})=>{
  await seedLibrary(page)
  await overrideApi(page,`
    const {listen}=await import('/src/lib/events.ts');
    await listen('nfo:export',event=>{if(event.completed===1)void api.jobCancel(event.jobId)});
    window.testApi=api;
  `)
  await page.goto('/')
  const before=await page.evaluate(async()=>{const api=(window as any).testApi;const codes=(await api.libraryList({limit:2})).items.map((m:any)=>m.code);return Promise.all(codes.map((code:string)=>api.libraryGet(code)))})
  await page.getByRole('button',{name:'媒体库工具',exact:true}).click()
  await page.getByRole('button',{name:'批量导出 NFO 语言',exact:true}).click()
  const batch=page.getByLabel('批量 NFO 语言导出',{exact:true})
  await batch.getByRole('checkbox').nth(0).check();await batch.getByRole('checkbox').nth(1).check()
  await batch.getByLabel('批量 NFO 语言',{exact:true}).selectOption('original')
  await batch.getByRole('button',{name:'预览批量 NFO 导出',exact:true}).click()
  await batch.getByRole('button',{name:'确认批量导出',exact:true}).click()
  await expect(batch).toContainText('已完成 1 部，任务已取消')
  const after=await page.evaluate(async codes=>Promise.all(codes.map((code:string)=>(window as any).testApi.libraryGet(code))),before.map(m=>m.meta.code))
  expect(after[0].meta.nfo_language).toBe('original');expect(after[1]).toEqual(before[1])
  await batch.getByRole('button',{name:'撤销本批导出',exact:true}).click()
  await expect(batch.getByRole('button',{name:'预览批量 NFO 导出',exact:true})).toBeVisible()
  expect(await page.evaluate(async code=>(window as any).testApi.libraryGet(code),before[0].meta.code)).toEqual(before[0])
})

for(const kind of ['artwork-open','artwork-preview','artwork-save'] as const){
  test(`${kind}: changed settings retire the dialog without committing a stale preview`,async({page})=>{
    await prepare(page,kind)
    await page.evaluate(async()=>{const {app}=await import('/src/lib/state.svelte.ts');app.settings={...app.settings!,min_interval_secs:7}})
    await expect(page.getByRole('dialog',{name:'裁剪海报'})).toHaveCount(0)
    await page.evaluate(()=>(window as any).editor.started(907))
    expect(await page.evaluate(()=>(window as any).editor.cancels)).toEqual(kind==='artwork-save'?[]:[907])
    await page.evaluate(()=>(window as any).editor.finish())
    expect(await page.evaluate(()=>(window as any).editor.saves)).toBe(kind==='artwork-save'?1:0)
    await expect(page.getByText('海报已保存并锁定，可从历史撤销')).toHaveCount(0)
  })
}
