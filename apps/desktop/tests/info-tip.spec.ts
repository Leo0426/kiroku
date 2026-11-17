import {test,expect} from '@playwright/test'
import {seedLibrary} from './helpers'

test('settings help stays beside its trigger despite animated ancestors',async({page})=>{
 await seedLibrary(page)
 await page.emulateMedia({reducedMotion:'no-preference'})
 await page.goto('/')
 await page.getByRole('navigation',{name:'主导航'}).getByRole('button',{name:'设置',exact:true}).click()
 await page.getByRole('tab',{name:'媒体服务器',exact:true}).click()
 const trigger=page.getByRole('button',{name:'Plex 友好命名说明',exact:true})
 await trigger.hover()
 const tip=page.locator('#'+await trigger.getAttribute('aria-describedby'))
 await expect(tip).toBeVisible()
 await expect.poll(async()=>{const a=await trigger.boundingBox(),b=await tip.boundingBox();return Math.abs(b!.y-(a!.y+a!.height))}).toBeLessThan(24)
 await expect.poll(async()=>{const a=await trigger.boundingBox(),b=await tip.boundingBox();return Math.abs(b!.x-a!.x)}).toBeLessThan(40)
 await page.setViewportSize({width:800,height:720})
 await trigger.hover()
 await expect.poll(async()=>{const b=await tip.boundingBox();return b!.x+b!.width}).toBeLessThanOrEqual(788)
 await trigger.focus()
 await page.keyboard.press('Escape')
 await expect(tip).not.toBeVisible()
})

for (const width of [800,1280]) {
 test(`settings help icons align with their switch labels at ${width}px`,async({page})=>{
  await seedLibrary(page)
  await page.setViewportSize({width,height:900})
  await page.goto('/')
  await page.getByRole('navigation',{name:'主导航'}).getByRole('button',{name:'设置',exact:true}).click()
  for (const tab of ['媒体服务器','整理','高级']) {
   await page.getByRole('tab',{name:tab,exact:true}).click()
   const rows=page.locator('.tipped:has(> label.switch)')
   expect(await rows.count()).toBeGreaterThan(0)
   for (const row of await rows.all()) {
    const label=await row.locator('label.switch > span').boundingBox()
    const icon=await row.getByRole('button').boundingBox()
    expect(Math.abs(label!.y+label!.height/2-icon!.y-icon!.height/2)).toBeLessThanOrEqual(1)
   }
  }
 })
}
