import {expect,test} from '@playwright/test'
import {overrideApi,seedLibrary} from './helpers'
test.beforeEach(async({page})=>{await seedLibrary(page);await overrideApi(page,'window.testApi=api;')})
test('explicit language export previews fallback, preserves paths and supports undo',async({page})=>{
 await page.goto('/');await page.locator('button.pcard').first().click();const panel=page.getByLabel('影片详情',{exact:true}),files=await panel.locator('.files').innerText()
 await panel.getByRole('button',{name:'语言与 NFO',exact:true}).click();await panel.getByLabel('NFO 导出语言',{exact:true}).selectOption('original');await panel.getByRole('button',{name:'预览 NFO 导出',exact:true}).click()
 await expect(panel.getByLabel('NFO 语言差异')).toContainText('原文简介未保存')
 await panel.getByRole('button',{name:'确认导出 NFO',exact:true}).click();await expect(panel.getByRole('button',{name:'撤销 NFO 导出'})).toBeVisible();expect(await panel.locator('.files').innerText()).toBe(files)
 await panel.getByRole('button',{name:'撤销 NFO 导出'}).click();expect(await panel.locator('.files').innerText()).toBe(files)
})
test('saving settings never implicitly exports existing NFO; batch preview apply undo is explicit',async({page})=>{
 await page.goto('/');await expect(page.locator('button.pcard').first()).toBeVisible();const codes=await page.evaluate(async()=>{const p=await (window as any).testApi.libraryList({limit:2});return p.items.map((m:any)=>m.code)})
 const result=await page.evaluate(async(codes)=>{const api=(window as any).testApi;const before=await api.libraryGet(codes[0]),s=await api.getSettings();s.nfo_language='original';await api.saveSettings(s);const after=await api.libraryGet(codes[0]);const p=await api.nfoExportPreview(codes,'zh-Hans');const r=await api.nfoExportApply(p.token,p.root);for(const item of r.results)await api.rollback(item.journalId);return {same:JSON.stringify(before)===JSON.stringify(after),warnings:p.items.map((m:any)=>m.warnings),count:r.results.length}},codes)
 expect(result.same).toBe(true);expect(result.count).toBe(2);expect(result.warnings[0].join(' ')).toContain('简体中文')
})
test('supplied language versions can be saved and selected while locked empty title wins',async({page})=>{
 await page.goto('/');await page.locator('button.pcard').first().click();const panel=page.getByLabel('影片详情',{exact:true}),code=await panel.locator('.head .code').innerText()
 await page.evaluate(async code=>{const api=(window as any).testApi,p=await api.revisionPreview(code,{title:''});await api.revisionSave(p.token,p.root,p.code)},code)
 await panel.getByRole('button',{name:'语言与 NFO',exact:true}).click();await panel.getByRole('button',{name:'保存已有语言文本',exact:true}).click();await panel.getByLabel('文本语言标签').fill('en-US');await panel.getByLabel('语言版本标题').fill('English');await panel.getByLabel('语言版本简介').fill('English plot');await panel.getByLabel('同时选为当前文本（锁定字段优先）').check();await panel.getByRole('button',{name:'预览保存语言版本'}).click();await panel.getByRole('button',{name:'确认导出 NFO',exact:true}).click()
 const meta=await page.evaluate(async code=>(await (window as any).testApi.libraryGet(code)).meta,code);expect(meta.title).toBe('');expect(meta.plot).toBe('English plot');expect(meta.text_versions.localized['en-US'].title).toBe('English')
})
test('batch language export UI selects previews applies and undoes films',async({page})=>{
 await page.goto('/');await page.getByRole('button',{name:'媒体库工具',exact:true}).click();await page.getByRole('button',{name:'批量导出 NFO 语言',exact:true}).click();const batch=page.getByLabel('批量 NFO 语言导出',{exact:true});await batch.getByRole('checkbox').nth(0).check();await batch.getByRole('checkbox').nth(1).check();await batch.getByLabel('批量 NFO 语言',{exact:true}).selectOption('original');await batch.getByRole('button',{name:'预览批量 NFO 导出',exact:true}).click();await expect(batch.getByLabel('批量 NFO 差异')).toContainText('原文简介未保存');await batch.getByRole('button',{name:'确认批量导出',exact:true}).click();await expect(batch).toContainText('已完成 2 部');await batch.getByRole('button',{name:'撤销本批导出',exact:true}).click();await expect(batch.getByRole('button',{name:'预览批量 NFO 导出',exact:true})).toBeVisible()
})
