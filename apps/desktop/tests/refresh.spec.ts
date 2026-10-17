import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'
test.beforeEach(async ({ page }) => seedLibrary(page))
test('updates cache, reselects source with preview, locks and unlocks without moving video', async ({ page }) => {
 await overrideApi(page,'window.testApi = api;')
 await page.goto('/'); await page.locator('button.pcard').first().click()
 const panel=page.getByLabel('影片详情',{exact:true}); const files=await panel.locator('.files').innerText(); const code=await panel.locator('.head .code').innerText();
 await panel.getByRole('button',{name:'更新与来源'}).click();
 await expect(panel.getByLabel('更新字段')).toHaveCount(10);
 await panel.getByRole('button',{name:'预览更新'}).click();
 await expect(panel.getByLabel('更新差异预览')).toBeVisible();
 await panel.getByRole('button',{name:'确认更新'}).click();
 await panel.getByRole('button',{name:'更新与来源'}).click();
 await panel.getByLabel('标题来源').selectOption('javdb');
 await panel.getByRole('button',{name:'预览标题来源'}).click();
 await expect(panel.getByLabel('更新差异预览')).toContainText('标题');
 await panel.getByRole('button',{name:'确认更新'}).click();
 let meta=await page.evaluate(async code=>(await (window as any).testApi.libraryGet(code)).meta,code);
 expect(meta.locked_fields).toContain('title'); expect(meta.provenance.title).toBe('javdb'); expect(meta.raw_sources.javdb.fields.title).toBe(meta.title);
 await panel.getByRole('button',{name:'更新与来源'}).click();
 await panel.getByRole('button',{name:'解锁标题',exact:true}).click(); await panel.getByRole('button',{name:'确认更新'}).click();
 meta=await page.evaluate(async code=>(await (window as any).testApi.libraryGet(code)).meta,code); expect(meta.locked_fields).not.toContain('title'); await expect(panel.locator('.files')).toHaveText(files);
})
test('late update preview cannot appear after detail closes',async({page})=>{
 await overrideApi(page,`const update=api.metadataUpdatePreview; api.metadataUpdatePreview=async(...args)=>{const p=await update(...args); await new Promise(r=>setTimeout(r,700)); return p;}`);
 await page.goto('/');await page.locator('button.pcard').first().click(); await page.getByRole('button',{name:'更新与来源'}).click();await page.getByRole('button',{name:'预览更新'}).click();await page.getByRole('button',{name:'关闭详情'}).click();await page.locator('button.pcard').nth(1).click();await page.waitForTimeout(900);await expect(page.getByLabel('更新差异预览')).toHaveCount(0);
})
test('locked empty title and unselected fields survive replacement, cancel and library switching invalidate candidates',async({page})=>{
 await overrideApi(page,'window.testApi = api;');await page.goto('/');await page.locator('button.pcard').first().click();const panel=page.getByLabel('影片详情',{exact:true});const code=await panel.locator('.head .code').innerText();
 const result=await page.evaluate(async code=>{
  const api=(window as any).testApi;const before=(await api.libraryGet(code)).meta;const manual=await api.revisionPreview(code,{title:null});await api.revisionSave(manual.token,manual.root,code);
  const next=await api.metadataUpdatePreview(code,{kind:'refresh',fields:['title','plot'],replace:true,force:true});const unchanged=(await api.libraryGet(code)).meta;await api.revisionSave(next.token,next.root,code);const after=(await api.libraryGet(code)).meta;
  const stale=await api.metadataUpdatePreview(code,{kind:'lock',field:'plot',locked:true});await api.setLibraryRoot('/mock/other');await api.setLibraryRoot('/mock/library');let message='';try{await api.revisionSave(stale.token,stale.root,code)}catch(e){message=String(e)}
  return {before,unchanged,after,message}
 },code);
 expect(result.unchanged.title).toBeNull();expect(result.after.title).toBeNull();expect(result.after.plot).not.toBe(result.before.plot);expect(result.after.studio).toBe(result.before.studio);expect(result.after.tags).toEqual(result.before.tags);expect(result.message).toContain('预览已过期');
})
test('refresh backend error is explicit and cancellation leaves library metadata unchanged',async({page})=>{
 await overrideApi(page,`api.metadataUpdatePreview=async()=>{throw new Error('cache refresh failed');}`);await page.goto('/');await page.locator('button.pcard').first().click();const panel=page.getByLabel('影片详情',{exact:true});const title=await panel.locator('h2').innerText();await panel.getByRole('button',{name:'更新与来源'}).click();await panel.getByRole('button',{name:'预览更新'}).click();await expect(panel.getByRole('alert')).toContainText('cache refresh failed');await panel.getByRole('button',{name:'取消更新',exact:true}).click();await expect(panel.locator('h2')).toHaveText(title);await expect(panel.getByLabel('更新差异预览')).toHaveCount(0);
})
test('source and refresh tokens expire after same-root source settings changes while manual editing survives',async({page})=>{
 await overrideApi(page,'window.testApi = api;');await page.goto('/');await page.locator('button.pcard').first().click();const code=await page.getByLabel('影片详情',{exact:true}).locator('.head .code').innerText();
 const result=await page.evaluate(async code=>{
  const api=(window as any).testApi;const original=await api.getSettings();const cache=await api.metadataUpdatePreview(code,{kind:'refresh',fields:[],replace:false,force:false});await api.revisionSave(cache.token,cache.root,code);const old=(await api.libraryGet(code)).meta;const failures=[];
  for(const kind of ['source','refresh'])for(const change of ['base','cookie','proxy','route','enabled']){
   await api.saveSettings(original);const p=await api.metadataUpdatePreview(code,kind==='source'?{kind,field:'title',source:'javdb'}:{kind,fields:['title'],replace:true,force:false});const changed=structuredClone(original);
   if(change==='base')changed.sources[0].base='https://new.example';if(change==='cookie')changed.sources[0].cookie='changed';if(change==='proxy')changed.proxy='http://localhost:9999';if(change==='route')Object.keys(changed.routes).forEach(k=>changed.routes[k]=[]);if(change==='enabled')changed.sources[0].enabled=false;await api.saveSettings(changed);
   try{await api.revisionSave(p.token,p.root,code);failures.push(`${kind}/${change} saved`)}catch(e){if(!String(e).includes('预览'))failures.push(String(e))}
  }
  const after=(await api.libraryGet(code)).meta;await api.saveSettings(original);const manual=await api.revisionPreview(code,{title:'manual survives'});const changed=structuredClone(original);changed.sources[0].cookie='same root edit';await api.saveSettings(changed);await api.revisionSave(manual.token,manual.root,code);return {failures,old,after,title:(await api.libraryGet(code)).meta.title}
 },code);expect(result.failures).toEqual([]);expect(result.after).toEqual(result.old);expect(result.title).toBe('manual survives');
})
test('source dropdown contains only eligible nonempty choices and refreshed source opens its new domain',async({page})=>{
 await overrideApi(page,'window.testApi = api;window.openedSources=[];api.openSourceUrl=async url=>window.openedSources.push(url);');await page.goto('/');await page.locator('button.pcard').first().click();const panel=page.getByLabel('影片详情',{exact:true});const code=await panel.locator('.head .code').innerText();
 await panel.getByRole('button',{name:'更新与来源'}).click();await panel.getByRole('button',{name:'预览更新'}).click();await panel.getByRole('button',{name:'确认更新'}).click();await panel.getByRole('button',{name:'更新与来源'}).click();await expect(panel.getByLabel('导演来源').locator('option')).toHaveCount(1);await expect(panel.getByLabel('标题来源').locator('option[value="javdb"]')).toHaveCount(1);
 await panel.getByRole('button',{name:'取消更新',exact:true}).click();await page.evaluate(async()=>{const api=(window as any).testApi;const settings=await api.getSettings();settings.sources[0].base='https://new.example';settings.sources[1].enabled=false;settings.field_priority.title=['javdb'];await api.saveSettings(settings)});await panel.getByRole('button',{name:'更新与来源'}).click();await expect(panel.getByLabel('标题来源').locator('option[value="javdb"]')).toHaveCount(0);await expect(panel.getByLabel('标题来源').locator('option[value="javbus"]')).toHaveCount(0);
 await panel.getByLabel('替换已有值').check();await panel.getByLabel('强制刷新原站缓存（重新联网）').check();await panel.getByRole('button',{name:'预览更新'}).click();await panel.getByRole('button',{name:'确认更新'}).click();
 // Select javdb as title priority to verify refresh rather than explicit reselect updates links.
 const url=await page.evaluate(async code=>(await (window as any).testApi.libraryGet(code)).meta.source_urls.javdb,code);expect(url).toContain('https://new.example/');await panel.locator('summary').click();await panel.getByRole('button',{name:'JavDB 原站'}).click();expect(await page.evaluate(()=>(window as any).openedSources)).toContain(url);
})
test('title-only refresh keeps actors and their projection when name rules change',async({page})=>{
 await overrideApi(page,'window.testApi = api;');await page.goto('/');await page.locator('button.pcard').first().click();const panel=page.getByLabel('影片详情',{exact:true});const code=await panel.locator('.head .code').innerText();
 const before=await page.evaluate(async code=>{const api=(window as any).testApi;const m=(await api.libraryGet(code)).meta;const settings=await api.getSettings();settings.name_rules.actors=[{canonical:'Changed by new rule',aliases:[m.actors[0].name]}];await api.saveSettings(settings);return m},code);
 await panel.getByRole('button',{name:'更新与来源'}).click();for(const checkbox of await panel.getByLabel('更新字段',{exact:true}).all())await checkbox.uncheck();await panel.locator('input[aria-label="更新字段"][value="title"]').check();await panel.getByLabel('替换已有值',{exact:true}).check();await panel.getByRole('button',{name:'预览更新',exact:true}).click();await expect(panel.getByLabel('更新差异预览')).not.toContainText('Changed by new rule');await panel.getByRole('button',{name:'确认更新',exact:true}).click();
 const results=await page.evaluate(async code=>{const api=(window as any).testApi;const after=(await api.libraryGet(code)).meta;const cache=await api.metadataUpdatePreview(code,{kind:'refresh',fields:[],replace:true,force:false});const preserve=await api.metadataUpdatePreview(code,{kind:'refresh',fields:['actors'],replace:false,force:false});return [after,cache.after,preserve.after]},code);for(const after of results){expect(after.actors).toEqual(before.actors);expect(after.name_projection??{}).toEqual(before.name_projection??{})}
})
