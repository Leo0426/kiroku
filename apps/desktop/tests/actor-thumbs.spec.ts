import {expect,test} from '@playwright/test'
import {overrideApi,seedLibrary} from './helpers'
test('actor chips show the scraped avatar and drop broken ones',async({page})=>{
 await seedLibrary(page)
 const dot='data:image/svg+xml,'+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4" fill="red"/></svg>')
 await overrideApi(page,`const get=api.libraryGet;api.libraryGet=async code=>{const d=await get(code);if(d){d.meta.actors=[{name:'有头像',gender:'female',thumb:${JSON.stringify(dot)}},{name:'坏头像',gender:'female',thumb:'https://invalid.invalid/x.jpg'},{name:'无头像',gender:'female'}]}return d};`)
 await page.goto('/');await page.locator('button.pcard').first().click()
 const chips=page.getByLabel('影片详情',{exact:true}).locator('.chips').first()
 await expect(chips.locator('.chip',{hasText:'有头像'}).locator('img.avatar')).toBeVisible()
 await expect(chips.locator('.chip',{hasText:'坏头像'}).locator('img')).toHaveCount(0)
 await expect(chips.locator('.chip',{hasText:'无头像'}).locator('img')).toHaveCount(0)
})
