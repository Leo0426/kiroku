import {expect,test} from '@playwright/test'
import {seedLibrary,overrideApi} from './helpers'

test('invalid NAS opt-in is visible before importing and never offers a probe',async({page})=>{
 await seedLibrary(page)
 await overrideApi(page,`api.nasStatus=async()=>({state:'invalid',message:'NAS 配置文件格式无效',localRoot:null,remoteRoot:null});`)
 await page.goto('/')
 await page.getByRole('navigation',{name:'主导航'}).getByRole('button',{name:'导入整理',exact:true}).click()
 const region=page.getByRole('region',{name:'NAS 文件操作'})
 await expect(region).toContainText('媒体写入已阻止')
 await expect(region).toContainText('nas-operations.json')
 await expect(region.getByRole('button',{name:'检测 NAS 文件操作'})).toHaveCount(0)
})

test('explicit NAS probe explains temporary writes and shows the result',async({page})=>{
 await seedLibrary(page)
 await overrideApi(page,`window.probes=0;api.nasStatus=async()=>({state:'unverified',message:'等待核验',localRoot:'/mock/mount',remoteRoot:'/media'});api.nasDiagnose=async()=>{window.probes++;return {state:'verified',message:'NAS 映射已核验',localRoot:'/mock/mount',remoteRoot:'/media'}};`)
 await page.goto('/')
 await page.getByRole('navigation',{name:'主导航'}).getByRole('button',{name:'设置',exact:true}).click()
 await page.locator('[data-tab="organize"]').click()
 const region=page.getByRole('region',{name:'NAS 文件操作'})
 await expect(region).toContainText('中性临时文件')
 expect(await page.evaluate(()=>(window as any).probes)).toBe(0)
 await region.getByRole('button',{name:'检测 NAS 文件操作'}).click()
 await expect(region).toContainText('映射已核验')
 expect(await page.evaluate(()=>(window as any).probes)).toBe(1)
})
