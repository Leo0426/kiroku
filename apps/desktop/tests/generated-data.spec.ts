import {expect,test} from '@playwright/test'
import {seedLibrary,overrideApi} from './helpers'

test('generated data can be individually trashed only after confirmation',async({page})=>{
 await seedLibrary(page)
 await overrideApi(page,`window.deleted=[];window.recent=false;api.maintenancePreview=async(includeRecent)=>{window.recent=includeRecent;return {token:'fresh',root:'/mock/library',logDir:'/mock/logs',policy:{historyDays:0,logDays:0},scannedAt:1,candidateBytes:10,protected:['有效撤销记录保留'],candidates:[{id:'original:unused',kind:'original',paths:['/mock/library/unused.png'],bytes:10,description:'无引用原图'}]}};api.maintenanceApply=async(token,ids)=>{window.deleted=ids;return {removed:1,skipped:[],failed:[],cancelled:false}};`)
 await page.goto('/');await page.getByRole('navigation',{name:'主导航'}).getByRole('button',{name:'设置',exact:true}).click();await page.getByRole('tab',{name:'生成数据',exact:true}).click()
 await page.getByRole('checkbox',{name:'包含近期可删除数据',exact:true}).check();await page.getByRole('button',{name:'预览空间清理',exact:true}).click();expect(await page.evaluate(()=>(window as any).recent)).toBe(true)
 await page.getByRole('button',{name:'删除 无引用原图',exact:true}).click();await page.getByRole('alertdialog').getByRole('button',{name:'取消',exact:true}).click();expect(await page.evaluate(()=>(window as any).deleted)).toEqual([])
 await page.getByRole('button',{name:'删除 无引用原图',exact:true}).click();await page.getByRole('alertdialog').getByRole('button',{name:'移到废纸篓',exact:true}).click();await expect(page.getByRole('status')).toContainText('已移到废纸篓');expect(await page.evaluate(()=>(window as any).deleted)).toEqual(['original:unused'])
})

for(const status of ['applied','rolledBack']) test(`${status} history is removed from display only and can be restored`,async({page})=>{
 await seedLibrary(page)
 await overrideApi(page,`window.deleted=false;api.historyList=async(_id,p={})=>{const items=(p.filter?.excludedIds??[]).includes('record')?[]:[{id:'record',ts:1780000000000,status:'${status}',itemCount:1,opCount:1,trashed:0,codes:['ABC-001'],pendingOperations:0,error:null}];return {items,total:items.length,next:null}};api.maintenanceApply=async()=>{window.deleted=true;throw 'must not delete'};`)
 await page.goto('/');await page.getByRole('button',{name:'历史',exact:true}).click()
 await page.getByRole('button',{name:'隐藏记录',exact:true}).click()
 await expect(page.getByRole('heading',{name:'当前没有可见记录'})).toBeVisible()
 await expect(page.getByRole('alertdialog')).toHaveCount(0)
 await expect(page.getByText('ABC-001',{exact:true})).toHaveCount(0)
 await page.reload();await page.getByRole('button',{name:'历史',exact:true}).click()
 await expect(page.getByText('ABC-001',{exact:true})).toHaveCount(0)
 await page.getByRole('button',{name:'显示已隐藏记录',exact:true}).click()
 await expect(page.getByText('ABC-001',{exact:true})).toBeVisible()
 expect(await page.evaluate(()=>(window as any).deleted)).toBe(false)
})

test('batch deletion reports changes and consumes the preview',async({page})=>{
 await seedLibrary(page)
 await overrideApi(page,`window.deleted=[];api.maintenancePreview=async()=>({token:'batch',root:'/mock/library',logDir:'/mock/logs',policy:{historyDays:0,logDays:0},scannedAt:1,candidateBytes:20,protected:['当前海报保留'],candidates:['a','b'].map(id=>({id:'original:'+id,kind:'original',paths:['/mock/library/'+id+'.png'],bytes:10,description:'无引用原图 '+id}))});api.maintenanceApply=async(token,ids)=>{window.deleted=ids;return {removed:1,skipped:['b 新增引用，已保留'],failed:[],cancelled:false}};`)
 await page.goto('/');await page.getByRole('button',{name:'历史',exact:true}).click();await page.getByRole('button',{name:'空间维护',exact:true}).click();await page.getByRole('button',{name:'预览空间清理',exact:true}).click();await page.getByRole('button',{name:'全选可删除项',exact:true}).click();await page.getByRole('button',{name:'清理选中项',exact:true}).click();await page.getByRole('alertdialog').getByRole('button',{name:'移到废纸篓',exact:true}).click();await expect(page.getByText('b 新增引用，已保留',{exact:true})).toBeVisible();expect(await page.evaluate(()=>(window as any).deleted)).toEqual(['original:a','original:b']);await expect(page.getByRole('button',{name:'清理选中项',exact:true})).toHaveCount(0)
})

test('changing library during delete confirmation revokes the old candidate',async({page})=>{
 await seedLibrary(page)
 await overrideApi(page,`window.deleted=false;api.maintenancePreview=async()=>({token:'old-root',root:'/mock/library',logDir:'/mock/logs',policy:{historyDays:0,logDays:0},scannedAt:1,candidateBytes:10,protected:[],candidates:[{id:'original:unused',kind:'original',paths:['/mock/library/unused.png'],bytes:10,description:'无引用原图'}]});api.maintenanceApply=async()=>{window.deleted=true;throw 'must not delete'};`)
 await page.goto('/');await page.getByRole('button',{name:'历史',exact:true}).click();await page.getByRole('button',{name:'空间维护',exact:true}).click();await page.getByRole('button',{name:'预览空间清理',exact:true}).click();await page.getByRole('button',{name:'删除 无引用原图',exact:true}).click();await expect(page.getByRole('alertdialog')).toBeVisible();
 await page.evaluate(async()=>{const {api}=await import('/src/lib/api.ts');const {app}=await import('/src/lib/state.svelte.ts');await api.setLibraryRoot('/mock/other',true);await app.refreshSettings()});
 await page.getByRole('alertdialog').getByRole('button',{name:'移到废纸篓',exact:true}).click();expect(await page.evaluate(()=>(window as any).deleted)).toBe(false);await expect(page.getByRole('button',{name:'删除 无引用原图',exact:true})).toHaveCount(0)
})
