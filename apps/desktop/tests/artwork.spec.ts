import { expect, test } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'
test.beforeEach(async ({ page }) => seedLibrary(page))
test('fixed crop frame drags over the original, saves, reopens at the same place and history restores without changing video', async ({ page }) => {
 await overrideApi(page,'window.testApi=api;');await page.goto('/');await page.locator('button.pcard').first().click()
 const panel=page.getByLabel('影片详情',{exact:true}),title=await panel.locator('h2').innerText(),files=await panel.locator('.files').innerText()
 await panel.getByRole('button',{name:'裁剪海报',exact:true}).click()
 const crop=page.getByRole('dialog',{name:'裁剪海报'});await expect(crop.getByAltText('裁剪原图')).toBeVisible()
 const position=crop.getByLabel('裁剪位置');await expect(position).toHaveText('X 377 · Y 0 · 423 × 600')
 // 拖到最左(越界被夹住),框的大小不变
 const frame=crop.getByRole('slider',{name:'裁剪框位置'}),box=(await frame.boundingBox())!
 await page.mouse.move(box.x+box.width/2,box.y+box.height/2);await page.mouse.down();await page.mouse.move(box.x-400,box.y+box.height/2,{steps:6});await page.mouse.up()
 await expect(position).toHaveText('X 0 · Y 0 · 423 × 600')
 await page.keyboard.press('ArrowRight');await page.keyboard.press('Shift+ArrowRight');await expect(position).toHaveText('X 11 · Y 0 · 423 × 600')
 await crop.getByRole('button',{name:'保存海报',exact:true}).click();await expect(crop).toHaveCount(0);await expect(panel.locator('h2')).toHaveText(title)
 await panel.getByRole('button',{name:'重新裁剪海报'}).click();await expect(crop.getByText('原图 800 × 600 像素')).toBeVisible();await expect(position).toHaveText('X 11 · Y 0 · 423 × 600')
 await page.keyboard.press('Escape');await expect(crop).toHaveCount(0);await expect(panel).toBeVisible();await expect(panel.locator('.files')).toHaveText(files)
 await panel.getByRole('button',{name:'关闭详情'}).click();await page.getByRole('navigation').getByRole('button',{name:'历史',exact:true}).click();await page.getByRole('button',{name:'撤销',exact:true}).first().click();await page.getByRole('button',{name:'确认撤销',exact:true}).click();await expect(page.getByRole('list', {name:'操作记录'}).getByText('已撤销',{exact:true})).toBeVisible()
})
test('right-click opens the same crop dialog from the poster wall',async({page})=>{
 await page.goto('/');const first=page.locator('button.pcard').first();const code=(await first.locator('.sub .mono').textContent())!.trim()
 await first.click({button:'right'});await page.getByRole('menuitem',{name:'裁剪海报…'}).click()
 const crop=page.getByRole('dialog',{name:'裁剪海报'});await expect(crop).toContainText(code);await crop.getByRole('slider',{name:'裁剪框位置'}).focus();await page.keyboard.press('Shift+ArrowLeft')
 await crop.getByRole('button',{name:'保存海报',exact:true}).click();await expect(crop).toHaveCount(0);await expect(page.getByText('海报已保存并锁定，可从历史撤销')).toBeVisible()
})
test('closing the crop dialog discards delayed original',async({page})=>{await overrideApi(page,`const open=api.artworkOpen;api.artworkOpen=async (...args)=>{const r=await open(...args);await new Promise(r=>setTimeout(r,600));return r};`);await page.goto('/');await page.locator('button.pcard').first().click();await page.getByRole('button',{name:'裁剪海报',exact:true}).click();await page.getByRole('button',{name:'关闭裁剪'}).click();await page.getByRole('button',{name:'关闭详情'}).click();await page.locator('button.pcard').nth(1).click();await page.waitForTimeout(800);await expect(page.getByAltText('裁剪原图')).toHaveCount(0)})
test('cancelled crop writes nothing and settings change rejects prepared crop',async({page})=>{
 await overrideApi(page,'window.testApi=api;');await page.goto('/');await page.locator('button.pcard').first().click();const panel=page.getByLabel('影片详情',{exact:true}),code=await panel.locator('.head .code').innerText()
 const before=await page.evaluate(async code=> (await (window as any).testApi.libraryGet(code)),code)
 await panel.getByRole('button',{name:'裁剪海报',exact:true}).click();const crop=page.getByRole('dialog',{name:'裁剪海报'});await crop.getByRole('slider',{name:'裁剪框位置'}).focus();await page.keyboard.press('Shift+ArrowLeft');await crop.getByRole('button',{name:'取消',exact:true}).click();await expect(crop).toHaveCount(0)
 expect(await page.evaluate(async code=>(await (window as any).testApi.libraryGet(code)),code)).toEqual(before)
 const error=await page.evaluate(async code=>{const api=(window as any).testApi;const o=await api.artworkOpen(code);const p=await api.artworkPreview(o.token,o.root,code,{x:1,y:1,width:200,height:300});const s=await api.getSettings();s.badges.enabled=!s.badges.enabled;await api.saveSettings(s);try{await api.revisionSave(p.revision.token,p.revision.root,code);return 'saved'}catch(e){return String(e)}},code)
 expect(error).toContain('设置已变化');expect(await page.evaluate(async code=>(await (window as any).testApi.libraryGet(code)),code)).toEqual(before)
})
test('switching away and back invalidates original tokens',async({page})=>{await overrideApi(page,'window.testApi=api;');await page.goto('/');const error=await page.evaluate(async()=>{const api=(window as any).testApi;const code=(await api.libraryList({})).items[0].code;const o=await api.artworkOpen(code);await api.setLibraryRoot('/other-library',true);await api.setLibraryRoot(o.root,true);try{await api.artworkPreview(o.token,o.root,code,{x:0,y:0,width:100,height:100});return 'accepted'}catch(e){return String(e)}});expect(error).not.toBe('accepted')})
