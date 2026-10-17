import {expect,test} from '@playwright/test'
import {seedLibrary} from './helpers'
test.beforeEach(async({page})=>{await seedLibrary(page);await page.goto('/')})
test('history cancellation is scoped and preserves early intent',async({page})=>{
 const result=await page.evaluate(async()=>{const{api}=await import('/src/lib/api.ts');await api.historyCancel('early');const early=await api.historyList('early').then(()=>'',String);const current=api.historyList('current');await api.historyCancel('other');const next=await current;const cancelled=api.historyList('stop');await api.historyCancel('stop');return{early,next,stopped:await cancelled.then(()=>'',String)}});
 expect(result.early).toContain('取消');expect(result.stopped).toContain('取消');expect(Array.isArray((result.next as any).items)).toBe(true)
})
test('old history requests are rejected across a root A B A switch',async({page})=>{
 const result=await page.evaluate(async()=>{const{api}=await import('/src/lib/api.ts');const settings=await api.getSettings();const pending=api.historyList('old-root').then(()=>'',String);await api.setLibraryRoot('/mock/other',true);await api.setLibraryRoot(settings.library_root!,true);return await pending});
 expect(result).toMatch(/取消|变化/)
})
test('leaving history cancels only its read and discards the late result',async({page})=>{
 await page.evaluate(async()=>{const{api}=await import('/src/lib/api.ts');const{app}=await import('/src/lib/state.svelte.ts');const state:any={cancelled:[]};(window as any).historyTest=state;api.historyList=async id=>new Promise(resolve=>{state.id=id;state.finish=()=>resolve({items:[{id:'stale',codes:['STALE'],ts:0,status:'applied',itemCount:1,opCount:1,trashed:0,pendingOperations:0,error:null}],total:1,next:null})});api.historyCancel=async id=>{state.cancelled.push(id);return true};app.navigate('history')});
 await expect(page.getByRole('button',{name:'停止读取'})).toBeVisible();await page.evaluate(async()=>{const{app}=await import('/src/lib/state.svelte.ts');app.navigate('library')});
 await expect.poll(()=>page.evaluate(()=>(window as any).historyTest.cancelled)).not.toEqual([]);await page.evaluate(()=>(window as any).historyTest.finish());await expect(page.getByText('STALE',{exact:true})).toHaveCount(0)
})
