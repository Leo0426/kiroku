import {expect,test} from '@playwright/test'
import {seedLibrary} from './helpers'
test('template context preview and file extension diagnostic',async({page})=>{
 await seedLibrary(page);await page.goto('/');await page.getByRole('navigation',{name:'主导航'}).getByRole('button',{name:'设置',exact:true}).click()
 await page.getByRole('tab',{name:'命名与 NFO',exact:true}).click()
 await page.getByLabel('目录模板',{exact:true}).fill('{prefix}/{suffix}/{actors}/{raw_dir}/{ext}')
 await expect(page.getByLabel('模板实时预览')).toContainText('SMPL/001/示例演员/示例文件夹/mp4')
 await page.getByLabel('文件名模板',{exact:true}).fill('{code}.{ext}')
 await expect(page.getByLabel('模板实时预览')).toContainText('请移除 {ext}')
 await page.getByRole('button',{name:/保存更改/}).click()
 // 预览里的同一条错误也是 alert;这里要确认的是保存被拒绝
 await expect(page.getByRole('alert',{name:'保存失败'})).toContainText('请移除 {ext}')
 await page.getByLabel('文件名模板',{exact:true}).fill('{codefake}')
 await expect(page.getByLabel('模板实时预览')).toContainText('未知占位符')
 await page.getByLabel('文件名模板',{exact:true}).fill('{code}[-CD{cd?}]')
 await expect(page.getByLabel('模板实时预览')).toContainText('SMPL-001-CD2.mp4')
})
