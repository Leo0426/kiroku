import {expect,test} from '@playwright/test'
import {overrideApi,seedLibrary} from './helpers'

test('actor avatars use backend bytes instead of hotlinking and keep names on failure',async({page})=>{
 await seedLibrary(page)
 let hotlinks=0
 await page.route('https://images.example.invalid/**',route=>{hotlinks++;return route.fulfill({status:403})})
 await overrideApi(page,`
 const get=api.libraryGet;
 api.libraryGet=async code=>{const d=await get(code);if(d)d.meta.actors=[
 {name:'有头像',gender:'female',thumb:'https://images.example.invalid/a.jpg'},
 {name:'坏头像',gender:'female',thumb:'https://images.example.invalid/b.jpg'},
 {name:'无头像',gender:'female'}];return d};
 api.actorImage=async(code,url)=>{
 if(url.endsWith('/b.jpg'))throw '头像下载失败';
 return {bytes:Array.from(Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII='),c=>c.charCodeAt(0))),mime:'image/png'};
 };
 `)
 await page.goto('/');await page.locator('button.pcard').first().click()
 const chips=page.getByLabel('影片详情',{exact:true}).locator('.chips').first()
 await expect(chips.locator('.chip',{hasText:'有头像'}).locator('img.avatar')).toBeVisible()
 await expect(chips.locator('.chip',{hasText:'有头像'}).locator('img.avatar')).toHaveAttribute('src',/^blob:/)
 await expect(chips.locator('.chip',{hasText:'坏头像'})).toBeVisible()
 await expect(chips.locator('.chip',{hasText:'坏头像'}).locator('img')).toHaveCount(0)
 await expect(chips.locator('.chip',{hasText:'无头像'}).locator('img')).toHaveCount(0)
 expect(hotlinks).toBe(0)
})
