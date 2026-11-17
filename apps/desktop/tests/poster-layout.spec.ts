import {test,expect} from '@playwright/test'
import {seedLibrary} from './helpers'

test('poster zoom is continuous and saved; row and column layout fits its viewport',async({page})=>{
 await seedLibrary(page)
 await page.goto('/')
 const card=page.locator('button.pcard').first()
 await expect(card).toBeVisible()
 const slider=page.getByRole('slider',{name:'海报缩放'})
 await slider.fill('60')
 const small=(await card.boundingBox())!.width
 await slider.fill('120')
 await expect.poll(async()=>(await card.boundingBox())!.width).toBeGreaterThan(small*1.8)
 await page.reload()
 await expect(slider).toHaveValue('120')
 await page.getByRole('button',{name:'海报布局',exact:true}).click()
 await page.getByLabel('海报排列方式').selectOption('grid')
 await page.getByLabel('海报列数').selectOption('4')
 await page.getByLabel('海报每屏行数').selectOption('2')
 const wall=page.locator('.scroller[data-columns]')
 await expect(wall).toHaveAttribute('data-columns','4')
 await expect.poll(()=>wall.evaluate(el=>Number((el as HTMLElement).dataset.rowHeight)*2<=el.clientHeight+1)).toBe(true)
 await page.reload()
 await page.getByRole('button',{name:'海报布局',exact:true}).click()
 await expect(page.getByLabel('海报列数')).toHaveValue('4')
 await expect(page.getByLabel('海报每屏行数')).toHaveValue('2')
 for(const [width,height] of [[390,600],[800,600],[960,500],[1440,900],[1920,1080]]){
  await page.setViewportSize({width,height})
  await expect.poll(()=>wall.evaluate(el=>el.clientHeight)).toBeGreaterThan(80)
  await expect.poll(()=>card.evaluate(el=>{const r=el.getBoundingClientRect();return r.left>=0&&r.right<=innerWidth+1})).toBe(true)
  expect(await page.locator('.view-shell').evaluate(el=>el.scrollWidth<=el.clientWidth)).toBe(true)
 }
 await page.getByRole('button',{name:'重置布局'}).click()
 await expect(slider).toHaveValue('100')
 await expect(page.getByLabel('海报排列方式')).toHaveValue('scale')
})
