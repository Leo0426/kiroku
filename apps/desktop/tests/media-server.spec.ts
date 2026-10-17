import {expect,test} from '@playwright/test'
import {seedLibrary} from './helpers'
test('jellyfin settings test the connection through the path prefix mapping',async({page})=>{
 await seedLibrary(page);await page.goto('/')
 await page.getByRole('navigation',{name:'主导航'}).getByRole('button',{name:'设置',exact:true}).click()
 await page.getByRole('tab',{name:/媒体服务器/}).click()
 const card=page.getByLabel('Jellyfin 与 Emby',{exact:true})
 await card.getByRole('checkbox',{name:/通知 Jellyfin \/ Emby/}).check()
 await card.getByRole('button',{name:'测试连接',exact:true}).click()
 await expect(card.getByRole('alert')).toContainText('API Key 无效')
 await card.getByLabel('API Key',{exact:true}).fill('k')
 await card.getByLabel('本机路径前缀',{exact:true}).fill('/mock')
 await card.getByLabel('服务器路径前缀',{exact:true}).fill('/media')
 await card.getByRole('button',{name:'测试连接',exact:true}).click()
 await expect(card.getByText('已连接 家里的服务器 10.11.0')).toBeVisible()
 await expect(card.locator('li.hit')).toContainText('/media/library')
 await page.getByRole('button',{name:/保存更改/}).click()
 await expect(page.getByRole('button',{name:/保存更改/})).toHaveCount(0)
})
