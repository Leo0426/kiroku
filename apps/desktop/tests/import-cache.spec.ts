import {expect,test} from '@playwright/test'
import {seedLibrary,overrideApi} from './helpers'

test('import cache explains restart reuse and clearing requires confirmation',async({page})=>{
 await seedLibrary(page)
 await overrideApi(page,`window.clearCalls=0;api.clearImportCache=async()=>{window.clearCalls++};`)
 await page.goto('/')
 await page.getByRole('navigation',{name:'主导航'}).getByRole('button',{name:'设置',exact:true}).click()
 await page.locator('[data-tab="data"]').click()
 const cache=page.getByRole('region',{name:'抓取缓存'})
 await expect(cache).toContainText('意外退出后')
 await cache.getByRole('button',{name:'清空抓取缓存',exact:true}).click()
 await expect(page.getByRole('alertdialog')).toContainText('媒体文件、整理日志和 AI 译文缓存不受影响')
 expect(await page.evaluate(()=>(window as any).clearCalls)).toBe(0)
 await page.getByRole('alertdialog').getByRole('button',{name:'清空缓存',exact:true}).click()
 await expect(page.getByText('抓取缓存已清空',{exact:true})).toBeVisible()
 expect(await page.evaluate(()=>(window as any).clearCalls)).toBe(1)
})
