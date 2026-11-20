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

test('manual actor avatars preview, save, and clear through the metadata editor', async ({ page }) => {
  await seedLibrary(page)
  await page.goto('/')
  await page.locator('button.pcard').first().click()
  const panel = page.getByLabel('影片详情', { exact: true })
  const editor = panel.getByLabel('人工编辑', { exact: true })
  const actor = panel.locator('.chip', { hasText: '示例演员' })
  await editor.getByRole('button', { name: '编辑元数据' }).click()
  await editor.getByRole('button', { name: '添加演员' }).click()
  await editor.getByLabel(/^演员姓名 /).last().fill('示例演员')
  const url = editor.getByLabel(/^演员头像网址 /).last()
  await url.fill('file:///example.png')
  await editor.getByRole('button', { name: '预览差异' }).click()
  await expect(editor.getByRole('alert')).toContainText('HTTP 或 HTTPS')
  await url.fill('https://images.example.invalid/perf.png')
  await editor.getByRole('button', { name: '预览差异' }).click()
  const preview = editor.getByLabel('修订差异预览')
  await expect(preview.locator('li', { hasText: '示例演员' }).locator('img')).toHaveAttribute('src', /^blob:/)
  await expect(actor).toHaveCount(0)
  await preview.getByRole('button', { name: '确认保存' }).click()
  await expect(actor.locator('img')).toHaveAttribute('src', /^blob:/)

  await editor.getByRole('button', { name: '编辑元数据' }).click()
  await expect(url).toHaveValue('https://images.example.invalid/perf.png')
  await expect(editor.locator('.actor-row').last().locator('img')).toBeVisible()
  await url.clear()
  await editor.getByRole('button', { name: '预览差异' }).click()
  await preview.getByRole('button', { name: '确认保存' }).click()
  await expect(actor).toBeVisible()
  await expect(actor.locator('img')).toHaveCount(0)
})
