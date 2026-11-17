import {expect,test} from '@playwright/test'
import {seedLibrary,overrideApi} from './helpers'
test('manual compact FZ identifier is accepted without padding and routes correctly',async({page})=>{
 await seedLibrary(page)
 await overrideApi(page,`const get=api.getSettings;api.getSettings=async()=>({...await get(),source_dirs:['/mock/source']});const set=api.setItemCode;api.setItemCode=async(...args)=>{const value=await set(...args);window.shortCode=value;return value}`)
 await page.goto('/')
 await page.getByRole('button',{name:'导入整理',exact:true}).click()
 await page.getByRole('button',{name:'开始扫描',exact:true}).click()
 await expect(page.getByRole('button',{name:/查找影片信息/})).toBeEnabled()
 const row=page.locator('.scanrow').filter({has:page.getByRole('textbox',{name:'手动输入番号'})}).first()
 await row.getByRole('textbox',{name:'手动输入番号'}).fill('FZ73')
 await row.getByRole('button',{name:'确定',exact:true}).click()
 await page.getByText(/查看已识别文件/).click()
 await expect(page.getByRole('button',{name:'修改番号 FZ73',exact:true})).toBeAttached()
 expect(await page.evaluate(()=>(window as any).shortCode.code)).toMatchObject({id:'FZ73',content_type:'uncensored',mosaic:'uncensored'})
})
